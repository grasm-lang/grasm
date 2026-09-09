#!/usr/bin/env bash
# HTTP/1.1 over TLS, against a real HTTPS server.
#
# `tls_diff.sh` proves the handshake; this proves the protocol on top of it,
# which is where the interesting failures are: a TLS read returns a record,
# and a record has no relationship to an HTTP message. Every case here is a
# different way a response ends.
#
#   /plain     a Content-Length body
#   /chunked   a chunked body, arriving in three chunks
#   /redirect  a 302 to /plain, which must be followed
#   /big       sixty thousand bytes, which arrives as several records
#   /missing   a 404, which is an answer rather than an error
#   /loop      a redirect to itself, which must stop rather than spin
#
# The server is Python's own `http.server` behind `ssl`, which negotiates
# ChaCha20-Poly1305 by preference — so this gate also covers the second
# record cipher, where `tls_diff.sh` covers the first.
set -uo pipefail
set +m
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
PY=${HTTPS_DIFF_PYTHON:-python3}
command -v "$PY" >/dev/null 2>&1 || { echo "python3 is needed"; exit 1; }
command -v openssl >/dev/null 2>&1 || { echo "openssl is needed"; exit 1; }

WORK=$(mktemp -d)
SERVER=""
cleanup() {
    if [ -n "$SERVER" ]; then kill "$SERVER" 2>/dev/null; wait 2>/dev/null; fi
    rm -rf "$WORK"
}
trap cleanup EXIT

"$GRASM" build packages/httpsdiff/src/main.gr --native -o "$WORK/httpsdiff" \
    >/dev/null 2>&1 || { echo "the client would not build"; exit 1; }
CLIENT="$WORK/httpsdiff"

openssl req -x509 -newkey rsa:2048 -keyout "$WORK/key.pem" \
    -out "$WORK/cert.pem" -days 30 -nodes -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost" >/dev/null 2>&1
openssl x509 -in "$WORK/cert.pem" -outform DER -out "$WORK/cert.der"

cat > "$WORK/server.py" <<'PYEOF'
import http.server, ssl, sys
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def do_GET(self):
        if self.path == "/plain":
            body = b"hello from the fixture\n"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers(); self.wfile.write(body)
        elif self.path == "/chunked":
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            for part in [b"one ", b"two ", b"three"]:
                self.wfile.write(b"%x\r\n%s\r\n" % (len(part), part))
            self.wfile.write(b"0\r\n\r\n")
        elif self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "/plain")
            self.send_header("Content-Length", "0"); self.end_headers()
        elif self.path == "/loop":
            self.send_response(302)
            self.send_header("Location", "/loop")
            self.send_header("Content-Length", "0"); self.end_headers()
        elif self.path == "/big":
            body = b"x" * 60000
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers(); self.wfile.write(body)
        else:
            self.send_response(404)
            self.send_header("Content-Length", "0"); self.end_headers()
port = int(sys.argv[1])
srv = http.server.ThreadingHTTPServer(("127.0.0.1", port), H)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(sys.argv[2], sys.argv[3])
ctx.set_alpn_protocols(["http/1.1"])
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
srv.serve_forever()
PYEOF

PORT=${HTTPSPORT:-44911}
"$PY" "$WORK/server.py" "$PORT" "$WORK/cert.pem" "$WORK/key.pem" \
    >/dev/null 2>&1 &
SERVER=$!
sleep 2

NOW=$(date +%s)
passed=0
failed=0
expect() {
    local name=$1 want=$2 got=$3
    case "$got" in
        $want) passed=$((passed + 1)) ;;
        *) failed=$((failed + 1)); echo "  $name: wanted $want, got $got" ;;
    esac
}

run() { "$CLIENT" "$PORT" "$WORK/cert.der" localhost "$1" "$NOW" 2>&1; }

expect "content-length" '200|OK|text/plain|23|hello from the fixture' "$(run /plain)"
expect "chunked"        '200|OK|text/plain|13|one two three'          "$(run /chunked)"
expect "redirect"       '200|OK|text/plain|23|hello from the fixture' "$(run /redirect)"
expect "many records"   '200|OK|application/octet-stream|60000|xxx*'  "$(run /big)"
expect "not found"      '404|Not Found||0|'                           "$(run /missing)"
expect "redirect loop"  'redirects|3'                                 "$(run /loop)"

echo "requests: $((passed + failed))   as expected: $passed   wrong: $failed"
[ "$failed" -eq 0 ]
