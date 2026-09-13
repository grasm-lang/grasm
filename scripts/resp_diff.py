"""A Grasm RESP server, driven by a client written here.

`redis-py` drove the same server during development — 28 checks, all
agreeing, and that is in the commit that added it. This is the part a clean
checkout can run: no gate needs `pip install`, and one that did would quietly
stop being run.

What it checks is the protocol rather than the storage: that a status is a
simple string and data is a bulk string, that a missing key is the *null*
bulk string rather than the empty one, that an integer reply is an integer,
and that an error is an error. Those are the four ways a server breaks a
client that reads it.
"""
import os
import re
import socket
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/resptest/src/main.gr")


def command(*parts):
    out = f"*{len(parts)}\r\n".encode()
    for one in parts:
        raw = one.encode() if isinstance(one, str) else one
        out += b"$%d\r\n%s\r\n" % (len(raw), raw)
    return out


class Reader:
    """Just enough RESP2 to judge a server with."""

    def __init__(self, sock):
        self.sock = sock
        self.buffer = b""

    def line(self):
        while b"\r\n" not in self.buffer:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise OSError("the server hung up")
            self.buffer += chunk
        line, _, rest = self.buffer.partition(b"\r\n")
        self.buffer = rest
        return line

    def exactly(self, n):
        while len(self.buffer) < n + 2:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise OSError("the server hung up")
            self.buffer += chunk
        out = self.buffer[:n]
        self.buffer = self.buffer[n + 2:]
        return out

    def reply(self):
        head = self.line()
        mark, rest = head[:1], head[1:]
        if mark == b"+":
            return ("status", rest.decode())
        if mark == b"-":
            return ("error", rest.decode())
        if mark == b":":
            return ("integer", int(rest))
        if mark == b"$":
            n = int(rest)
            if n == -1:
                return ("nil", None)
            return ("bulk", self.exactly(n))
        if mark == b"*":
            n = int(rest)
            return ("array", [self.reply() for _ in range(n)])
        raise OSError(f"a reply this does not understand: {head[:20]!r}")


def main():
    server = subprocess.Popen([GRASM, "run", HARNESS, "--", "serve"],
                              stdout=subprocess.PIPE, text=True)
    first = server.stdout.readline()
    found = re.search(r"port (\d+)", first)
    if not found:
        print(f"the server did not say a port: {first!r}")
        server.kill()
        return 1
    sock = socket.create_connection(("127.0.0.1", int(found.group(1))),
                                    timeout=30)
    sock.settimeout(30)
    reader = Reader(sock)
    checks = []

    def check(name, sent, want):
        sock.sendall(command(*sent))
        got = reader.reply()
        checks.append((name, got == want, got, want))

    big = "x" * 100000
    check("ping is a status", ["PING"], ("status", "PONG"))
    check("echo is a bulk string", ["ECHO", "hello"], ("bulk", b"hello"))
    check("set is a status", ["SET", "k", "v"], ("status", "OK"))
    check("get is a bulk string", ["GET", "k"], ("bulk", b"v"))
    check("a missing key is the null bulk string, not the empty one",
          ["GET", "nothing"], ("nil", None))
    check("exists is an integer", ["EXISTS", "k", "nothing"], ("integer", 1))
    check("incr from nothing", ["INCR", "n"], ("integer", 1))
    check("incrby", ["INCRBY", "n", "9"], ("integer", 10))
    check("decrby", ["DECRBY", "n", "4"], ("integer", 6))
    sock.sendall(command("INCR", "k"))
    kind, text = reader.reply()
    checks.append(("counting a word is an error", kind == "error", kind,
                   "error"))
    check("append answers the new length", ["APPEND", "k", "w"],
          ("integer", 2))
    check("unicode survives", ["SET", "u", "café 日本語"], ("status", "OK"))
    check("and comes back as the same bytes", ["GET", "u"],
          ("bulk", "café 日本語".encode()))
    check("a hundred thousand bytes go in", ["SET", "big", big],
          ("status", "OK"))
    check("and come back", ["GET", "big"], ("bulk", big.encode()))
    check("type", ["TYPE", "k"], ("status", "string"))
    check("type of a missing key", ["TYPE", "gone"], ("status", "none"))
    check("dbsize", ["DBSIZE"], ("integer", 4))
    check("delete answers how many went", ["DEL", "n", "gone"],
          ("integer", 1))
    check("flushall", ["FLUSHALL"], ("status", "OK"))
    check("dbsize after flush", ["DBSIZE"], ("integer", 0))
    sock.sendall(command("NOPE"))
    kind, _ = reader.reply()
    checks.append(("an unknown command is an error", kind == "error", kind,
                   "error"))
    # Two commands in one write must produce two replies in order: a server
    # that reads one packet as one command loses the second.
    sock.sendall(command("SET", "p", "1") + command("GET", "p"))
    first_reply = reader.reply()
    second_reply = reader.reply()
    checks.append(("two commands in one write get two replies",
                   (first_reply, second_reply) ==
                   (("status", "OK"), ("bulk", b"1")),
                   (first_reply, second_reply), "OK then 1"))
    sock.sendall(command("QUIT"))
    sock.close()
    try:
        server.wait(timeout=30)
    except subprocess.TimeoutExpired:
        server.kill()
    said = (server.stdout.read() or "").strip()
    wrong = [c for c in checks if not c[1]]
    for name, _ok, got, want in wrong[:6]:
        print(f"  {name}: got {got!r}, wanted {want!r}")
    print(f"checks: {len(checks)}   agreeing: {len(checks) - len(wrong)}   "
          f"differing: {len(wrong)}   server said: {said}")
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
