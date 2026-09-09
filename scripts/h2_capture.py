#!/usr/bin/env python3
"""Record what a real HTTP/2 server sends, so a gate can replay it.

`h2_diff.sh` may not depend on anything outside Python's standard library —
every other gate is runnable on a clean checkout, and one that needs `pip
install` is one that quietly stops being run. But an HTTP/2 server written
here to test a client written here proves only that the two agree with each
other, which is the failure this project exists to avoid.

So the independent implementation is consulted once, here, and its output is
committed. This script needs `h2` and `hpack` (`pip install h2`); the gate
does not. What it writes down is every byte the `h2` library sent, in order,
including the HPACK header blocks — which are the interesting part, because
they are encoded against a dynamic table that three responses down one
connection keep changing.

Each line of the fixture is a frame the server sent, and the number of
client requests that had arrived when it sent it:

    <requests-seen> <credit-seen> raw <hex>
    <requests-seen> <credit-seen> fill <hex of the 9-byte header> <byte> <n>

`credit-seen` is how many bytes of connection-level WINDOW_UPDATE the client
had sent by then. Replaying a frame before its credit arrives would let a
client that never refills the connection window pass.

`fill` is a DATA frame whose payload is one byte repeated, written short so
a forty-thousand-byte response does not become eighty thousand characters of
fixture. The expansion is checked here against the bytes it replaces.

Run: scripts/h2_capture.py scripts/fixtures/h2-transcript.txt
"""
import os
import socket
import ssl
import subprocess
import sys
import tempfile
import threading

try:
    import h2.config
    import h2.connection
    import h2.events
except ImportError:
    sys.exit("this needs the `h2` package: pip install h2")

# The three responses the gate asks for, in the order it asks for them.
PATHS = ["/", "/missing", "/big"]
# Two hundred thousand bytes is deliberately more than the sixty-five
# thousand of a default connection window: the server cannot finish sending
# it until the client refills, so replaying this proves the client refills.
BODIES = {"/": b"hello over h2\n", "/big": b"y" * 200000}


def split_frames(data):
    """The frames in a buffer, which `data_to_send` returns several of."""
    out = []
    at = 0
    while at + 9 <= len(data):
        size = int.from_bytes(data[at:at + 3], "big")
        out.append(data[at:at + 9 + size])
        at += 9 + size
    assert at == len(data), "a partial frame came out of data_to_send"
    return out


def record(frame, seen, credit, lines):
    kind = frame[3]
    payload = frame[9:]
    # A DATA frame of one repeated byte is written short.
    if kind == 0 and len(payload) > 64 and len(set(payload)) == 1:
        header = frame[:9].hex()
        line = f"{seen} {credit} fill {header} {payload[0]:02x} " \
            f"{len(payload)}"
        assert bytes.fromhex(header) + bytes([payload[0]]) * len(payload) \
            == frame, "the short form does not expand to the frame"
        lines.append(line)
    else:
        lines.append(f"{seen} {credit} raw {frame.hex()}")


def serve(tls, lines, done):
    conn, _ = tls.accept()
    seen = 0
    credit = 0
    pending = {}
    config = h2.config.H2Configuration(client_side=False)
    c = h2.connection.H2Connection(config=config)
    c.initiate_connection()
    flush(c, conn, pending, seen, credit, lines)
    try:
        while True:
            data = conn.recv(65535)
            if not data:
                break
            for event in c.receive_data(data):
                if isinstance(event, h2.events.WindowUpdated) and \
                        event.stream_id == 0:
                    credit += event.delta
                if not isinstance(event, h2.events.RequestReceived):
                    continue
                seen += 1
                path = dict(event.headers).get(b":path", b"/").decode()
                body = BODIES.get(path)
                if body is None:
                    c.send_headers(event.stream_id, [
                        (":status", "404"), ("content-length", "0")],
                        end_stream=True)
                    continue
                c.send_headers(event.stream_id, [
                    (":status", "200"), ("content-type", "text/plain"),
                    ("content-length", str(len(body)))])
                pending[event.stream_id] = body
            flush(c, conn, pending, seen, credit, lines)
    except Exception:
        pass
    finally:
        conn.close()
        done.set()


def flush(c, conn, pending, seen, credit, lines):
    """Send what is queued, honouring the client's flow-control window."""
    for stream in list(pending):
        body = pending[stream]
        while body:
            window = min(c.local_flow_control_window(stream),
                         c.max_outbound_frame_size)
            if window <= 0:
                break
            chunk, body = body[:window], body[window:]
            c.send_data(stream, chunk, end_stream=not body)
        if body:
            pending[stream] = body
        else:
            pending.pop(stream, None)
    out = c.data_to_send()
    if not out:
        return
    for frame in split_frames(out):
        record(frame, seen, credit, lines)
    conn.sendall(out)


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: h2_capture.py <fixture path>")
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    work = tempfile.mkdtemp()
    cert, key = os.path.join(work, "cert.pem"), os.path.join(work, "key.pem")
    subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048",
                    "-keyout", key, "-out", cert, "-days", "30", "-nodes",
                    "-subj", "/CN=localhost",
                    "-addext", "subjectAltName=DNS:localhost"],
                   check=True, capture_output=True)
    der = os.path.join(work, "cert.der")
    subprocess.run(["openssl", "x509", "-in", cert, "-outform", "DER",
                    "-out", der], check=True, capture_output=True)

    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(cert, key)
    ctx.set_alpn_protocols(["h2"])
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", 0))
    port = listener.getsockname()[1]
    listener.listen(1)
    tls = ctx.wrap_socket(listener, server_side=True)

    lines = []
    done = threading.Event()
    threading.Thread(target=serve, args=(tls, lines, done), daemon=True).start()

    client = os.path.join(work, "h2diff")
    grasm = os.environ.get("GRASM", os.path.join(root, "target/release/grasm"))
    subprocess.run([grasm, "build", "packages/h2diff/src/main.gr",
                    "--native", "-o", client], cwd=root, check=True,
                   capture_output=True)
    now = str(int(subprocess.run(["date", "+%s"], capture_output=True,
                                 text=True).stdout.strip()))
    got = subprocess.run([client, str(port), der, "localhost", now] + PATHS,
                         capture_output=True, text=True, timeout=60)
    done.wait(10)
    print(got.stdout, end="")
    if got.stderr:
        print(got.stderr, end="", file=sys.stderr)

    header = [
        "# Every byte a real HTTP/2 server sent, in order.",
        "#",
        "# Written by scripts/h2_capture.py against the Python `h2` library,",
        "# whose HPACK encoder produced the header blocks below. `h2_diff.sh`",
        "# replays these at the client so the gate needs nothing but the",
        "# standard library, and so the bytes the client is judged against",
        "# are an outside implementation's rather than this project's.",
        "#",
        "# <requests seen> <connection window credit seen> raw  <frame hex>",
        "# <requests seen> <credit seen> fill <header hex> <byte> <count>",
        "#",
        "# The requests, in order: " + " ".join(PATHS),
    ]
    with open(sys.argv[1], "w") as f:
        f.write("\n".join(header + lines) + "\n")
    print(f"{len(lines)} frames -> {sys.argv[1]}")


main()
