"""B28 against its references.

- `winpath` against CPython's own `ntpath`, on paths assembled from drive
  letters, UNC and device prefixes, both separators, dots, reserved names
  and non-ASCII letters.
- `txtar` against golang.org/x/tools/txtar: Parse, and Format of what was
  parsed, on archives with near-miss marker lines and \\r\\n endings.
- `httpproxy` against golang.org/x/net/http/httpproxy's ProxyFunc.
- `bpf` against golang.org/x/net/bpf: Disassemble's text, NewVM's
  validation and Run's answer, on programs mixing well-formed and random
  opcodes, run over random packets.
- `proxyproto` against pires/go-proxyproto: what Read accepts, its fields,
  how many bytes it took, and Format.
- `maptile` against paulmach/orb/maptile, floats to within 1e-8 degrees.

The Go side is scripts/lib/gomods, built offline from the module cache.
"""
import ntpath
import json
import os
import random
import subprocess
import sys
import tempfile

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts/lib/gomods"))
import oracle as gomods  # noqa: E402

rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="b28diff")
harness = os.path.join(work, "b28test")
built = subprocess.run([GRASM, "build", "packages/b28test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
oracle = gomods.build(os.path.join(work, "gomods"))
U = "\x1f"
cases = []          # (op, a, b, check)
go_requests = []    # filled in, then answered in one batch
go_slots = []       # (case index, request index, finisher)


def add(op, a, b, want):
    cases.append((op, a, b, lambda got: None if got == want else f"want {want[:300]}"))


# --- winpath ---------------------------------------------------------------------
TOKENS = ["C:", "c:", "Z:", "\\", "/", "\\\\", "//", "server", "share", "dir", "a.b", ".", "..", "...", "file.txt",
          " ", "x", "\\\\?\\", "\\\\.\\", "UNC\\", "unc/", "?", ":", "CON", "nul.txt", "com1", "COM\u00b9", "aux ",
          "lpt9.log", "\u00e9", "\u00c4", ".hidden", "name.", "tar.gz", "\t", "conin$", "Conout$.x"]


def winpath_string():
    return "".join(rng.choice(TOKENS) for _ in range(rng.randrange(0, 7)))


def b(x):
    return "true" if x else "false"


for _ in range(1500):
    p = winpath_string()
    fn = rng.choice(["splitroot", "split", "splitext", "splitdrive", "isabs", "normpath", "normcase", "basename",
                     "isreserved", "join", "commonpath", "relpath"])
    if fn == "splitroot":
        add("win.splitroot", p, "-", U.join(ntpath.splitroot(p)))
    elif fn in ("split", "splitext", "splitdrive"):
        add("win." + fn, p, "-", U.join(getattr(ntpath, fn)(p)))
    elif fn == "isabs":
        add("win.isabs", p, "-", b(ntpath.isabs(p)))
    elif fn == "normpath":
        add("win.normpath", p, "-", ntpath.normpath(p))
    elif fn == "normcase":
        add("win.normcase", p, "-", ntpath.normcase(p))
    elif fn == "basename":
        add("win.basename", p, "-", ntpath.basename(p) + U + ntpath.dirname(p))
    elif fn == "isreserved":
        add("win.isreserved", p, "-", b(ntpath.isreserved(p)))
    elif fn == "join":
        parts = [winpath_string() for _ in range(rng.randrange(1, 5))]
        add("win.join", U.join(parts), "-", ntpath.join(*parts))
    elif fn == "commonpath":
        root = rng.choice(["C:\\", "c:/", "\\\\srv\\sh\\", "", "\\", "D:"])
        parts = [root + "\\".join(rng.choice(["a", "A", "b", ".", "c d", "..", ""]) for _ in range(rng.randrange(0, 5)))
                 for _ in range(rng.randrange(1, 4))]
        if rng.random() < 0.2:
            parts.append(winpath_string())
        try:
            want = ntpath.commonpath(parts)
        except ValueError:
            want = "ERR"
        add("win.commonpath", U.join(parts), "-", want)
    else:
        roots = ["C:\\", "c:/", "D:\\", "\\\\srv\\sh\\", "\\\\SRV\\SH\\"]
        a = rng.choice(roots) + "\\".join(rng.choice(["a", "A", "b", ".", "..", "x y"]) for _ in range(rng.randrange(0, 5)))
        c = rng.choice(roots) + "\\".join(rng.choice(["a", "A", "b", ".", ".."]) for _ in range(rng.randrange(0, 5)))
        try:
            want = ntpath.relpath(a, c)
        except ValueError:
            want = "ERR"
        add("win.relpath", a + U + c, "-", want)


# UNC and device paths, which random tokens rarely spell out whole.
PREFIXES = ["\\\\?\\UNC\\", "\\\\?\\unc\\", "//?/UNC/", "\\\\.\\", "\\\\server\\share", "//srv/sh",
            "\\\\?\\C:\\", "\\\\", "\\\\x"]
for _ in range(400):
    p = rng.choice(PREFIXES) + "\\".join(rng.choice(["srv", "share", "a", "..", ".", "b.c", ""]) for _ in range(rng.randrange(0, 5)))
    fn = rng.choice(["splitroot", "split", "normpath", "isabs", "join"])
    if fn == "splitroot":
        add("win.splitroot", p, "-", U.join(ntpath.splitroot(p)))
    elif fn == "split":
        add("win.split", p, "-", U.join(ntpath.split(p)))
    elif fn == "normpath":
        add("win.normpath", p, "-", ntpath.normpath(p))
    elif fn == "isabs":
        add("win.isabs", p, "-", b(ntpath.isabs(p)))
    else:
        q = rng.choice(["x", "\\y", "C:z", "..\\w"])
        add("win.join", p + U + q, "-", ntpath.join(p, q))


# --- the Go-checked families -------------------------------------------------------
def via_go(op, a, bb, request, finish=lambda x: x):
    cases.append((op, a, bb, None))
    go_slots.append((len(cases) - 1, len(go_requests), finish))
    go_requests.append(request)


LINES = ["-- a.txt --", "-- dir/b.go --", "--  --", "-- x", "x -- y --", "-- spaced name --", "--\tname --",
         "-- \u00a0nbsp\u3000 --", "text", "", "more text", "-- c --\r", "--- d ---", "-- e --  ", "\r"]
for _ in range(250):
    lines = [rng.choice(LINES) for _ in range(rng.randrange(0, 10))]
    sep = "\r\n" if rng.random() < 0.2 else "\n"
    data = sep.join(lines) + (sep if rng.random() < 0.6 else "")
    raw = data.encode()
    via_go("txtar", raw.hex(), "-", {"op": "txtar", "hex": raw.hex()})

PROXIES = ["", "", "proxy:3128", "http://p.example:8080", "socks5://s.example:1080", "HTTP://Up.example:1",
           "https://secure.example:443/path"]
NO = ["example.com", ".example.com", "*.example.com", "Example.COM:8080", "10.0.0.0/8", "192.168.1.1", "[::1]:80",
      "::1", "2001:db8::/32", "*", " spaced.org ", "foo.bar:443", "127.0.0.2", "1.2.3.4:80", "", "localhost",
      "[fe80::1]", "a:b:c", "::ffff:10.0.0.0/104", "10.0.0.1/32", "sub.foo.bar", ".org"]
HOSTS = ["example.com", "a.example.com", "EXAMPLE.com", "notexample.com", "localhost", "LOCALHOST", "127.0.0.1",
         "10.1.2.3", "192.168.1.1", "[::1]", "[2001:db8::1]", "[::ffff:10.0.0.1]", "1.2.3.4", "foo.bar",
         "user@host.org", "spaced.org", "x.sub.foo.bar", "fe80--1.example", "[2001:db9::1]", "something.org"]
for _ in range(250):
    cfg = [rng.choice(PROXIES), rng.choice(PROXIES), ",".join(rng.choice(NO) for _ in range(rng.randrange(0, 5))),
           "1" if rng.random() < 0.15 else "0"]
    urls = []
    for _ in range(8):
        scheme = rng.choice(["http", "https", "https", "ftp", "HTTP"])
        port = rng.choice(["", ":80", ":443", ":8080"])
        urls.append(f"{scheme}://{rng.choice(HOSTS)}{port}{rng.choice(['', '/', '/a?b=c', '#f'])}")
    via_go("proxy", U.join(cfg), U.join(urls), {"op": "proxy", "args": cfg, "keys": urls})


def ins(op, jt, jf, k):
    return bytes([op & 255, op >> 8, jt, jf]) + (k & 0xFFFFFFFF).to_bytes(4, "little")


def bpf_program():
    n = rng.randrange(1, 12)
    out = b""
    for i in range(n):
        room = n - i - 1
        r = rng.random()
        if r < 0.12:
            out += ins(rng.choice([0x20, 0x28, 0x30, 0x40, 0x48, 0x50, 0x21, 0x29]), 0, 0, rng.randrange(0, 40))
        elif r < 0.2:
            out += ins(rng.choice([0x00, 0x01, 0x60, 0x61, 0x02, 0x03, 0x80, 0xb1, 0x07, 0x87]), 0, 0, rng.randrange(0, 18))
        elif r < 0.4:
            op = 0x04 | rng.choice([0x00, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xa0]) | rng.choice([0, 8])
            out += ins(op, 0, 0, rng.choice([0, 1, 2, 7, 31, 32, 33, 255, 0xFFFFFFFF, rng.getrandbits(32)]))
        elif r < 0.6:
            op = 0x05 | rng.choice([0x00, 0x10, 0x20, 0x30, 0x40]) | rng.choice([0, 8])
            out += ins(op, rng.randrange(0, room + 2), rng.randrange(0, room + 2),
                       rng.choice([0, 1, 0x800, 6, 17, rng.getrandbits(32), rng.randrange(0, room + 2)]))
        elif r < 0.7:
            out += ins(0x20, 0, 0, 0xFFFFF000 + rng.choice([0, 1, 4, 52, 56, 60, 100, 0xfff]))
        elif r < 0.8:
            out += ins(rng.getrandbits(16), rng.getrandbits(8), rng.getrandbits(8), rng.getrandbits(32))
        else:
            out += ins(rng.choice([0x06, 0x16]), 0, 0, rng.choice([0, 1, 96, 65535, 0xFFFFFFFF]))
    if rng.random() < 0.8:
        out += ins(rng.choice([0x06, 0x16]), 0, 0, rng.choice([0, 65535, 0xFFFFFFFF]))
    return out


for _ in range(400):
    prog = bpf_program()
    packets = [bytes(rng.getrandbits(8) for _ in range(rng.randrange(0, 40))).hex() for _ in range(4)]
    via_go("bpf", prog.hex(), ",".join(packets), {"op": "bpf", "hex": prog.hex(), "hexes": packets})

# Programs that compute and return A, so every ALU operation and load
# width is answered for, not just decoded.
for _ in range(400):
    prog = ins(0x00, 0, 0, rng.getrandbits(32)) + ins(0x01, 0, 0, rng.choice([0, 1, 5, 31, 32, 40, rng.getrandbits(32)]))
    if rng.random() < 0.3:
        prog += ins(rng.choice([0x20, 0x28, 0x30, 0x40, 0x48, 0x50]), 0, 0, rng.randrange(0, 12))
    for _ in range(rng.randrange(1, 4)):
        op = 0x04 | rng.choice([0x00, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x90, 0xa0]) | rng.choice([0, 8])
        prog += ins(op, 0, 0, rng.choice([1, 2, 3, 7, 31, 32, 33, 4000, 0xFFFFFFFF, rng.getrandbits(32)]))
    if rng.random() < 0.3:
        prog += ins(0x15 | rng.choice([0, 0x10, 0x20, 0x30]), 1, 0, rng.getrandbits(32)) + ins(0x06, 0, 0, 7)
    prog += ins(0x16, 0, 0, 0)
    packets = [bytes(rng.getrandbits(8) for _ in range(rng.randrange(0, 20))).hex() for _ in range(3)]
    via_go("bpf", prog.hex(), ",".join(packets), {"op": "bpf", "hex": prog.hex(), "hexes": packets})

SIG2 = bytes([13, 10, 13, 10, 0, 13, 10, 81, 85, 73, 84, 10])


def tlv_blob():
    out = b""
    for _ in range(rng.randrange(0, 4)):
        kind = rng.choice([1, 2, 3, 4, 5, 0x20, 0x30, 0xe0])
        value = bytes(rng.getrandbits(8) for _ in range(rng.randrange(0, 8)))
        out += bytes([kind]) + len(value).to_bytes(2, "big") + value
    if rng.random() < 0.15:
        out += bytes([1, 0])
    return out


def v2_header():
    cmd = rng.choice([0x21, 0x21, 0x20, 0x22, 0x11])
    fam = rng.choice([0x11, 0x12, 0x21, 0x22, 0x31, 0x32, 0x00, 0x13])
    if fam in (0x11, 0x12):
        body = bytes(rng.getrandbits(8) for _ in range(12))
    elif fam in (0x21, 0x22):
        body = bytes(rng.getrandbits(8) for _ in range(36))
    elif fam in (0x31, 0x32):
        names = [rng.choice([b"/tmp/a.sock", b"/run/x", b""]) for _ in range(2)]
        body = b"".join(n + bytes(108 - len(n)) for n in names)
    else:
        body = b""
    body += tlv_blob()
    length = len(body)
    if rng.random() < 0.1:
        length = rng.choice([0, 5, len(body) + 3, 5000])
    data = SIG2 + bytes([cmd, fam]) + length.to_bytes(2, "big") + body
    if rng.random() < 0.1:
        data = data[:rng.randrange(0, len(data) + 1)]
    return data


def v1_header():
    v4 = [".".join(str(rng.randrange(256)) for _ in range(4)), "01.2.3.4", "256.1.1.1", "::1"]
    v6 = ["2001:db8::1", "::1", "::ffff:1.2.3.4", "1.2.3.4", "fe80::1%eth0", "1:2:3:4:5:6:7:8", "1:2:3:4:5:6:7::"]
    fam = rng.choice(["TCP4", "TCP6", "UNKNOWN", "UDP4", "tcp4"])
    pool = v4 if fam in ("TCP4", "tcp4") else v6
    ports = [str(rng.randrange(65536)), "0", "080", "65536", ""]
    parts = ["PROXY", fam, rng.choice(pool), rng.choice(pool), rng.choice(ports), rng.choice(ports)]
    if rng.random() < 0.1:
        parts = parts[:rng.randrange(1, 6)]
    line = " ".join(parts) + rng.choice(["\r\n", "\r\n", "\n", ""])
    if rng.random() < 0.05:
        line = "PROXY TCP4 " + "1" * 120 + "\r\n"
    return line.encode()


for _ in range(400):
    r = rng.random()
    data = v1_header() if r < 0.45 else v2_header() if r < 0.95 else bytes(rng.getrandbits(8) for _ in range(20))
    data += rng.choice([b"", b"GET / HTTP/1.1\r\n", bytes(range(10))])
    via_go("pp", data.hex(), "-", {"op": "pp", "hex": data.hex()})

for _ in range(300):
    lon = rng.uniform(-180, 179.999999)
    lat = rng.choice([rng.uniform(-85, 85), rng.uniform(-89.9, 89.9), 85.06, -85.06, 0.0])
    z = rng.randrange(0, 21)
    via_go("tile", f"{lon!r},{lat!r},{z}", "-", {"op": "tile", "nums": [lon, lat, z]})


def tile_check(want):
    def check(got):
        g, w = got.split(" "), want.split(" ")
        if len(g) != len(w) + 1 or g[-1] != "true":
            return f"want {want} (and a quadkey round trip)"
        for x, y in zip(g, w):
            if "," in y and "." in y and any(c.isdigit() for c in y) and y.count(".") >= 2 and "-" not in y.split(",")[0][1:]:
                pass
            try:
                fx, fy = [float(v) for v in x.split(",")], [float(v) for v in y.split(",")]
                if len(fx) == len(fy) and all(abs(a - c) < 1e-8 for a, c in zip(fx, fy)):
                    continue
            except ValueError:
                pass
            if x != y:
                return f"field {x} != {y}; want {want}"
        return None
    return check


ran = subprocess.run([oracle], input=json.dumps(go_requests), capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-1500:]); sys.exit(1)
answers = json.loads(ran.stdout)
for ci, ri, finish in go_slots:
    op, a, bb, _ = cases[ci]
    want = finish(answers[ri])
    if op == "tile":
        cases[ci] = (op, a, bb, tile_check(want))
    elif op == "bpf" and ";neg;" in ";" + want.split("/")[0] + ";":
        # x/net/bpf's VM has no case for `neg` and fails on it; the kernel,
        # and the package, negate A. Hold the text to Go and the runs to the
        # kernel's rule through the demo instead.
        texts = want.split("/")[0]
        cases[ci] = (op, a, bb, (lambda t: lambda got: None if got.split("/")[0] == t else f"want {t}")(texts))
    else:
        cases[ci] = (op, a, bb, (lambda w: lambda got: None if got == w else f"want {w[:300]}")(want))


def run(picked):
    argv = []
    for op, a, bb, _ in picked:
        argv += [op, a, bb]
    lines, start = [], 0
    while start < len(argv):
        chunk = argv[start:start + 600]
        start += 600
        try:
            out = subprocess.run([harness] + chunk, capture_output=True, text=True, timeout=300)
        except subprocess.TimeoutExpired:
            print("the harness did not finish in 300 s"); sys.exit(1)
        if out.returncode != 0:
            print(out.stderr[-2000:]); sys.exit(1)
        lines += out.stdout.split("\n")[:-1]
    return lines


got = run(cases)
families = {}
agree = differ = 0
for i, (op, a, bb, check) in enumerate(cases):
    mine = got[i] if i < len(got) else "<missing>"
    why = check(mine) if mine != "<missing>" else "missing"
    fam = families.setdefault(op.split(".")[0], [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        only = os.environ.get("ONLY", "")
        if op.startswith(only) and (differ <= 10 or only):
            print(f"  {op} {a[:150]!r} {bb[:100]!r}")
            print(f"    grasm {mine[:300]!r}")
            print(f"    {why[:300]!r}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 6:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
