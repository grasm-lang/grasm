#!/usr/bin/env bash
# HTTP/2, against bytes a real HTTP/2 server produced.
#
# `https_diff.sh` proves HTTP/1.1 over TLS. This proves the protocol that
# replaced it, where a response is not a stream of text but a sequence of
# frames whose headers are compressed against every header block sent before
# them on the same connection.
#
# The server here is a replayer, not an implementation: it hands the client
# `scripts/fixtures/h2-transcript.txt`, which is every byte Python's `h2`
# library sent when `scripts/h2_capture.py` drove this same client at it.
# That keeps the gate inside the standard library — no other gate needs
# `pip install`, and one that did would quietly stop being run — while still
# judging the client against somebody else's encoder.
#
# Three requests go down one connection, because that is what a second
# connection would hide:
#
#   /          a short body, and the first header block, which primes the
#              HPACK dynamic table
#   /missing   a 404, whose block is mostly references into that table
#   /big       two hundred thousand bytes, which is more than a default
#              connection window holds, so the server stops until the client
#              refills it — the deadlock a half-written client produces
#
# Each recorded frame carries the number of requests and the number of bytes
# of connection-level WINDOW_UPDATE that had arrived when the real server
# sent it, and the replayer waits for both. A client that never refills the
# window therefore hangs here rather than passing.
set -uo pipefail
set +m
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
PY=${H2_DIFF_PYTHON:-python3}
FIXTURE=scripts/fixtures/h2-transcript.txt
command -v "$PY" >/dev/null 2>&1 || { echo "python3 is needed"; exit 1; }
command -v openssl >/dev/null 2>&1 || { echo "openssl is needed"; exit 1; }
[ -r "$FIXTURE" ] || { echo "$FIXTURE is missing"; exit 1; }

WORK=$(mktemp -d)
SERVER=""
cleanup() {
    if [ -n "$SERVER" ]; then kill "$SERVER" 2>/dev/null; wait 2>/dev/null; fi
    rm -rf "$WORK"
}
trap cleanup EXIT

"$GRASM" build packages/h2diff/src/main.gr --native -o "$WORK/h2diff" \
    >/dev/null 2>&1 || { echo "the client would not build"; exit 1; }
CLIENT="$WORK/h2diff"

openssl req -x509 -newkey rsa:2048 -keyout "$WORK/key.pem" \
    -out "$WORK/cert.pem" -days 30 -nodes -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost" >/dev/null 2>&1
openssl x509 -in "$WORK/cert.pem" -outform DER -out "$WORK/cert.der"

cat > "$WORK/replay.py" <<'PYEOF'
"""Hand a client a recorded HTTP/2 conversation, at the right moments."""
import socket, ssl, sys

PREFACE = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"


def load(path):
    out = []
    for line in open(path):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        seen, credit, kind = int(parts[0]), int(parts[1]), parts[2]
        if kind == "raw":
            out.append((seen, credit, bytes.fromhex(parts[3])))
        else:
            head, fill, count = parts[3], int(parts[4], 16), int(parts[5])
            out.append((seen, credit, bytes.fromhex(head) +
                        bytes([fill]) * count))
    return out


def serve(conn, script):
    """Read the client's frames, sending each recorded one when it is due."""
    at, seen, credit, buffer, started = 0, 0, 0, b"", False
    while at < len(script):
        while at < len(script) and script[at][0] <= seen \
                and script[at][1] <= credit:
            conn.sendall(script[at][2])
            at += 1
        if at >= len(script):
            break
        conn.settimeout(10)
        piece = conn.recv(65535)
        if not piece:
            return "the client closed before the transcript ran out"
        buffer += piece
        if not started:
            if len(buffer) < len(PREFACE):
                continue
            if not buffer.startswith(PREFACE):
                return "the client did not send the connection preface"
            buffer, started = buffer[len(PREFACE):], True
        # Count what the client sends: HEADERS opens a request, and a
        # WINDOW_UPDATE on stream zero is the credit the transcript waits on.
        while len(buffer) >= 9:
            size = int.from_bytes(buffer[0:3], "big")
            if len(buffer) < 9 + size:
                break
            frame, buffer = buffer[:9 + size], buffer[9 + size:]
            kind = frame[3]
            stream = int.from_bytes(frame[5:9], "big") & 0x7FFFFFFF
            if kind == 0x01:
                seen += 1
            elif kind == 0x08 and stream == 0:
                credit += int.from_bytes(frame[9:13], "big")
    return ""


port, cert, key, fixture = int(sys.argv[1]), sys.argv[2], sys.argv[3], \
    sys.argv[4]
script = load(fixture)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(cert, key)
ctx.set_alpn_protocols(["h2"])
listener = socket.socket()
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", port))
listener.listen(1)
tls = ctx.wrap_socket(listener, server_side=True)
conn, _ = tls.accept()
try:
    trouble = serve(conn, script)
    if trouble:
        print(trouble, file=sys.stderr)
    # Let the client read the last frames and send its GOAWAY.
    conn.settimeout(5)
    try:
        while conn.recv(65535):
            pass
    except Exception:
        pass
finally:
    conn.close()
PYEOF

PORT=${H2PORT:-44913}
"$PY" "$WORK/replay.py" "$PORT" "$WORK/cert.pem" "$WORK/key.pem" \
    "$FIXTURE" >"$WORK/server.log" 2>&1 &
SERVER=$!
sleep 2

NOW=$(date +%s)
"$CLIENT" "$PORT" "$WORK/cert.der" localhost "$NOW" / /missing /big \
    >"$WORK/got.txt" 2>&1

passed=0
failed=0
expect() {
    local name=$1 want=$2 got=$3
    case "$got" in
        $want) passed=$((passed + 1)) ;;
        *) failed=$((failed + 1)); echo "  $name: wanted $want, got $got" ;;
    esac
}

expect "short body"   '200|text/plain|14|hello over h2'      "$(sed -n 1p "$WORK/got.txt")"
expect "not found"    '404||0|'                              "$(sed -n 2p "$WORK/got.txt")"
expect "window refill" '200|text/plain|200000|yyyyyyyyyy*'   "$(sed -n 3p "$WORK/got.txt")"
expect "no fourth line" ''                                   "$(sed -n 4p "$WORK/got.txt")"

if [ -s "$WORK/server.log" ]; then
    failed=$((failed + 1))
    echo "  the replayer complained: $(head -1 "$WORK/server.log")"
fi

echo "responses: $((passed + failed))   as expected: $passed   wrong: $failed"
[ "$failed" -eq 0 ]
