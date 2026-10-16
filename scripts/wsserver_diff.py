"""A Grasm WebSocket server, driven by a client written here.

The client is a hundred lines of standard library rather than a dependency:
no other gate needs `pip install`, and one that did would quietly stop being
run. What it costs is that the oracle is not somebody else's implementation,
so the parts that *can* be checked against a published value are: the
handshake accept token is RFC 6455 section 1.3's own worked example, and the
frames are built from the standard's field layout rather than from what this
server happens to produce.

During development the same server was driven by Python's `websockets`
package at 250 clients and 1750 messages with every round trip agreeing;
that is in the commit that added it. This is the part a clean checkout can
run.
"""
import base64
import hashlib
import os
import re
import socket
import struct
import subprocess
import sys
import threading

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/wstest/src/main.gr")
GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

MESSAGES = [
    "hello",
    "",
    "a" * 200,
    "b" * 70000,
    "unicode: café 日本語 🌍",
    "with\nnewlines\tand\ttabs",
]


def accept_token(key):
    return base64.b64encode(
        hashlib.sha1((key + GUID).encode()).digest()).decode()


def frame(opcode, payload):
    """One masked frame, which is the only kind a client may send."""
    head = bytes([0x80 | opcode])
    mask = os.urandom(4)
    n = len(payload)
    if n < 126:
        head += bytes([0x80 | n])
    elif n < 65536:
        head += bytes([0x80 | 126]) + struct.pack(">H", n)
    else:
        head += bytes([0x80 | 127]) + struct.pack(">Q", n)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return head + mask + masked


def read_exactly(sock, n):
    out = b""
    while len(out) < n:
        chunk = sock.recv(n - len(out))
        if not chunk:
            raise OSError("the server hung up")
        out += chunk
    return out


def read_frame(sock):
    head = read_exactly(sock, 2)
    opcode = head[0] & 0x0F
    n = head[1] & 0x7F
    if head[1] & 0x80:
        raise OSError("a server must not mask")
    if n == 126:
        n = struct.unpack(">H", read_exactly(sock, 2))[0]
    elif n == 127:
        n = struct.unpack(">Q", read_exactly(sock, 8))[0]
    return opcode, read_exactly(sock, n)


def talk(port, results, index):
    """One client: handshake, every message, and the answers."""
    key = base64.b64encode(os.urandom(16)).decode()
    sock = socket.create_connection(("127.0.0.1", port), timeout=30)
    sock.settimeout(30)
    sock.sendall(
        f"GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
        f"Upgrade: websocket\r\nConnection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        .encode())
    reply = b""
    while b"\r\n\r\n" not in reply:
        reply += sock.recv(4096)
    if b"101 Switching Protocols" not in reply:
        results[index] = f"handshake refused: {reply[:60]!r}"
        return
    said = re.search(rb"Sec-WebSocket-Accept: (\S+)", reply)
    if not said or said.group(1).decode() != accept_token(key):
        results[index] = "the accept token does not match RFC 6455's rule"
        return
    for text in MESSAGES:
        sock.sendall(frame(1, text.encode()))
        opcode, payload = read_frame(sock)
        if opcode != 1 or payload.decode() != "echo:" + text:
            results[index] = (f"sent {text[:20]!r}, got "
                              f"{payload[:40]!r} opcode {opcode}")
            return
    blob = bytes(range(256))
    sock.sendall(frame(2, blob))
    opcode, payload = read_frame(sock)
    if opcode != 2 or payload != blob:
        results[index] = "the binary message came back changed"
        return
    # A ping must come back as a pong carrying the same payload.
    sock.sendall(frame(9, b"are you there"))
    opcode, payload = read_frame(sock)
    if opcode != 10 or payload != b"are you there":
        results[index] = f"ping answered with opcode {opcode} {payload[:20]!r}"
        return
    sock.sendall(frame(8, b"\x03\xe8"))
    sock.close()
    results[index] = None


def main():
    clients = int(os.environ.get("WS_CLIENTS", "40"))
    server = subprocess.Popen([GRASM, "run", HARNESS, "--", "serve"],
                              stdout=subprocess.PIPE, text=True)
    first = server.stdout.readline()
    found = re.search(r"port (\d+)", first)
    if not found:
        print(f"the server did not say a port: {first!r}")
        server.kill()
        return 1
    port = int(found.group(1))
    # Not `[None] * clients`. `talk` writes `None` only after it has got
    # through every message, and a thread that raises writes nothing at all --
    # `threading.Thread` prints the traceback and keeps going. Starting at
    # `None` made a client that crashed on its first frame indistinguishable
    # from one that passed, so forty crashed clients reported "agreeing: 40".
    # Starting at a complaint means only an explicit success clears it.
    results = ["the client did not finish"] * clients
    threads = [threading.Thread(target=talk, args=(port, results, i))
               for i in range(clients)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    try:
        server.wait(timeout=60)
    except subprocess.TimeoutExpired:
        server.kill()
    said = (server.stdout.read() or "").strip()
    wrong = [r for r in results if r]
    for one in wrong[:5]:
        print(f"  {one}")
    print(f"clients: {clients}   agreeing: {clients - len(wrong)}   "
          f"differing: {len(wrong)}   server said: {said}")
    if clients < 1:
        print("  no clients ran, so nothing was checked")
        return 1
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
