"""S2, CRC-32C, NAT-PMP and Canonical XML.

- `s2compress`: data of every texture (text, random, runs, fixed-stride
  records that make repeat offsets pay) compressed by klauspost/compress's
  four block encoders and its stream writer (S2 and Snappy framing, with
  and without its seek index) and read here; blocks and streams written
  here read by klauspost (and the Snappy-compatible blocks by
  `packages/snappy` too); hand-built blocks for every repeat length
  class; and damaged blocks and streams that both must judge the same.
- `crc32c`: against Go's hash/crc32 Castagnoli table, and the framing
  format's mask written out in Python.
- `natpmp`: go-nat-pmp's client sends each request to a responder on the
  loopback interface that answers with the case's bytes, well-formed or
  not; the requests built here must be the bytes it sent, and the answers
  read here must be what it made of them.
- `xmlc14n`: random documents (namespaces declared, redeclared and
  undeclared, attributes needing every escape, character references, CDATA,
  comments and processing instructions inside and outside the document
  element, XML declarations, DOCTYPEs) and broken ones, against
  `xmllint --c14n` and `--exc-c14n`; without comments, against xmllint on
  the same document written without them.
"""
import json
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
sys.path.insert(0, os.path.join(ROOT, "scripts/lib/gomods"))
import oracle  # noqa: E402

rng = random.Random(20261019)
work = tempfile.mkdtemp(prefix="b28c")
harness = os.path.join(work, "b28test3")
built = subprocess.run([GRASM, "build", "packages/b28test3/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
go = oracle.build(os.path.join(work, "gomods"))


def ask(reqs):
    out = subprocess.run([go], input=json.dumps(reqs), capture_output=True, text=True)
    if out.returncode != 0:
        print(out.stderr[-2000:]); sys.exit(1)
    return json.loads(out.stdout)


def rb(n):
    return bytes(rng.randrange(256) for _ in range(n))


cases = []   # [op, a, b, want, go request, how to read the answer]


def add(op, a, b=b"", want=None, req=None, then=None):
    cases.append([op, a, b, want, req, then])


# --- S2 ----------------------------------------------------------------------
WORDS = [b"the ", b"quick ", b"brown ", b"fox ", b"jumps ", b"over ", b"lazy ", b"dog ", b"\n", b"compression "]


def texture():
    kind = rng.choice(["text", "random", "runs", "stride", "mixed", "tiny"])
    n = rng.choice([0, 1, 5, 17, 100, 1000, 5000, 40000, 70000, 200000])
    if kind == "text":
        out = b""
        while len(out) < n:
            out += rng.choice(WORDS)
        return out[:n]
    if kind == "random":
        return rb(min(n, 20000))
    if kind == "runs":
        out = b""
        while len(out) < n:
            out += bytes([rng.randrange(256)]) * rng.randrange(1, 3000)
        return out[:n]
    if kind == "stride":
        rec = rb(rng.randrange(4, 40))
        out = b""
        i = 0
        while len(out) < n:
            out += rec[:2] + struct.pack("<I", i) + rec[6:]
            i += 1
        return out[:n]
    if kind == "mixed":
        return texture() + rb(rng.randrange(0, 50)) + texture()
    return rb(rng.randrange(0, 4))


datas = [texture() for _ in range(220)]
for d in datas:
    for mode in ["default", "better", "best", "snappy"]:
        add("s2.dec", b"", want=d.hex(), req={"op": "s2.enc", "hex": d.hex(), "args": [mode]}, then="encoded")
    add("s2.enc", d, then=("decodes", d))
    add("s2.encsnappy", d, then=("decodes", d))
    add("s2.stream.enc", d, str(rng.choice([1024, 4096, 65536, 1 << 20])).encode(), then=("stream", d))
    # Given nothing and asked for an index, klauspost writes an index chunk
    # with no identifier, which its own reader refuses.
    opt = rng.choice(["", "snappy", "index"] if d else ["", "snappy"])
    size = rng.choice([4096, 65536, 1 << 20]) if opt != "snappy" else rng.choice([4096, 65536])
    add("s2.stream.dec", b"", want=d.hex(), req={"op": "s2.stream.enc", "hex": d.hex(), "args": [str(size), opt]}, then="encoded")


def literal(data):
    n = len(data) - 1
    if n < 60:
        return bytes([n << 2]) + data
    return bytes([60 << 2, n]) + data if n < 256 else bytes([61 << 2]) + struct.pack("<H", n) + data


def uvarint(v):
    out = b""
    while v >= 128:
        out += bytes([v & 127 | 128])
        v >>= 7
    return out + bytes([v])


# Hand-built blocks: a literal, a copy, then repeats of every length class.
for _ in range(300):
    seed = rb(rng.randrange(8, 40))
    body = literal(seed)
    out = bytearray(seed)
    off = rng.randrange(1, len(seed) + 1)
    n = rng.randrange(4, 12)
    body += bytes([(off >> 8) << 5 | (n - 4) << 2 | 1, off & 255]) if off < 2048 else b""
    for _ in range(n):
        out.append(out[-off])
    for _ in range(rng.randrange(1, 4)):
        L = rng.choice([4, 5, 8, 9, 100, 263, 264, 1000, 65795, 65796, 70000])
        if L <= 8:
            body += bytes([(L - 4) << 2 | 1, 0])
        elif L < 264:
            body += bytes([5 << 2 | 1, 0, L - 8])
        elif L < 65796:
            body += bytes([6 << 2 | 1, 0]) + struct.pack("<H", L - 260)
        else:
            body += bytes([7 << 2 | 1, 0]) + struct.pack("<I", L - 65540)[:3]
        for _ in range(L):
            out.append(out[-off])
        if rng.random() < 0.3:
            lit = rb(rng.randrange(1, 5))
            body += literal(lit)
            out += lit
    block = uvarint(len(out)) + body
    r = rng.random()
    if r < 0.15:
        block = uvarint(len(out) + rng.choice([1, -1])) + body
    elif r < 0.25:
        block = uvarint(len(out)) + bytes([1, 0]) + body
    add("s2.dec", block, req={"op": "s2.dec", "hex": block.hex()}, then="same")

# Damaged blocks and streams.
damaged_src = [texture() for _ in range(160)]
enc = ask([{"op": "s2.enc", "hex": d.hex(), "args": [rng.choice(["default", "better", "snappy"])]} for d in damaged_src])
streams = ask([{"op": "s2.stream.enc", "hex": d.hex(), "args": [str(rng.choice([4096, 65536])), rng.choice(["", "snappy", "index"])]}
               for d in damaged_src])
for hx, sx in zip(enc, streams):
    b = bytearray(bytes.fromhex(hx))
    if len(b) > 2:
        if rng.random() < 0.5:
            b[rng.randrange(len(b))] ^= 1 << rng.randrange(8)
        else:
            b = b[:rng.randrange(1, len(b))]
    add("s2.dec", bytes(b), req={"op": "s2.dec", "hex": bytes(b).hex()}, then="same")
    s = bytearray(bytes.fromhex(sx))
    r = rng.random()
    if r < 0.2 and len(s) > 14:
        s[rng.randrange(10, len(s))] ^= 1 << rng.randrange(8)
    elif r < 0.35 and len(s) > 1:
        s = s[:rng.randrange(1, len(s) + 1)]
    elif r < 0.45:
        s = s[10:]
    elif r < 0.55:
        s = s[:10] + bytes([2, 3, 0, 0, 1, 2, 3]) + s[10:]
    elif r < 0.7:
        s = s[:10] + bytes([rng.choice([0x80, 0x99, 0xfe]), 4, 0, 0]) + rb(4) + s[10:]
    elif r < 0.8:
        s = s + bytes.fromhex(sx)
    elif r < 0.9:
        s = bytes([0xff, 6, 0, 0]) + b"S2sTwX" + s[10:]
    add("s2.stream.dec", bytes(s), req={"op": "s2.stream.dec", "hex": bytes(s).hex()}, then="same")
add("s2.stream.dec", b"", req={"op": "s2.stream.dec", "hex": ""}, then="same")

# --- CRC-32C -----------------------------------------------------------------


def mask(c):
    return (((c >> 15) | (c << 17)) + 0xA282EAD8) & 0xFFFFFFFF


for n in list(range(0, 40)) + [rng.randrange(0, 5000) for _ in range(300)]:
    d = rb(n) if rng.random() < 0.7 else b"123456789"[:n]
    add("crc32c", d, req={"op": "crc32c", "hex": d.hex()}, then="crc")

# --- NAT-PMP -----------------------------------------------------------------
for _ in range(500):
    if rng.random() < 0.4:
        spec = ["ext"]
        good = bytes([0, 128, 0, 0]) + struct.pack(">I", rng.randrange(1 << 32)) + rb(4)
        size = 12
    else:
        proto = rng.choice(["udp", "tcp"])
        spec = ["map", proto, str(rng.randrange(65536)), str(rng.choice([0, rng.randrange(65536)])),
                str(rng.choice([0, 3600, 7200, rng.randrange(1 << 31)]))]
        op = 1 if proto == "udp" else 2
        good = bytes([0, 128 + op, 0, 0]) + struct.pack(">IHHI", rng.randrange(1 << 32), rng.randrange(65536),
                                                         rng.randrange(65536), rng.randrange(1 << 32))
        size = 16
    r = rng.random()
    answer = good
    if r < 0.1:
        answer = good[:rng.randrange(1, size)]
    elif r < 0.18 and size == 12:
        # go-nat-pmp reads into 16 bytes, so a longer mapping answer is cut to
        # size there; a longer external-address answer is refused by both.
        answer = good + rb(rng.randrange(1, 4))
    elif r < 0.26:
        answer = bytes([rng.randrange(1, 3)]) + good[1:]
    elif r < 0.34:
        answer = good[:1] + bytes([good[1] ^ rng.choice([1, 2, 128])]) + good[2:]
    elif r < 0.45:
        answer = good[:2] + struct.pack(">H", rng.randrange(1, 6)) + good[4:]
    add("natpmp.req", b"", "|".join(spec).encode(), req={"op": "natpmp", "args": spec, "hex": answer.hex()}, then="natreq")
    add("natpmp.ans", answer, b"", then="natans")

# --- Canonical XML -----------------------------------------------------------
PREFIXES = ["p", "q", "ns1", "x"]
# Absolute URIs only: Canonical XML fails on relative namespace names (a
# few documents get one on purpose, below). No `&`: libxml2 writes it bare
# in a namespace declaration, where the specification escapes it.
URIS = ["urn:a", "urn:b", "http://example.com/ns", "urn:a%26b", "tag:x.org,2026:y"]
BAD_URIS = ["rel/path", "urn:é", "urn:a b", "#frag", "urn:%zz"]


def escape_text(s):
    out = ""
    for c in s:
        r = rng.random()
        if c == "&":
            out += rng.choice(["&amp;", "&#38;", "&#x26;"])
        elif c == "<":
            out += rng.choice(["&lt;", "&#60;"])
        elif c == ">":
            out += rng.choice(["&gt;", "&#62;"])
        elif c == "\r":
            out += rng.choice(["&#13;", "&#xD;"])
        elif r < 0.05 and c.isalpha():
            out += f"&#{ord(c)};"
        else:
            out += c
    return out


def text_value():
    return "".join(rng.choice(["a", "b", " ", "\t", "\n", "&", "<", ">", "\"", "'", "\r", "é", "€", "xyz"])
                   for _ in range(rng.randrange(0, 8)))


def attr_value(q):
    out = ""
    for c in text_value():
        if c == "&":
            out += "&amp;"
        elif c == "<":
            out += "&lt;"
        elif c == q:
            out += "&quot;" if q == '"' else "&apos;"
        elif c in "\t\n\r" and rng.random() < 0.5:
            out += f"&#{ord(c)};"
        elif c == "\r":
            out += "&#13;"
        else:
            out += c
    if rng.random() < 0.1:
        out += "\r\n"
    return out


class Doc:
    def __init__(self):
        self.parts_c, self.parts_nc = [], []

    def put(self, s, comment=False):
        self.parts_c.append(s)
        if not comment:
            self.parts_nc.append(s)


def element(doc, depth, scope):
    scope = dict(scope)
    decls = []
    for _ in range(rng.randrange(0, 3)):
        r = rng.random()
        if r < 0.2:
            decls.append(("", rng.choice(URIS)))
        elif r < 0.3:
            decls.append(("", ""))
        elif r < 0.45 and scope:
            p = rng.choice(list(scope))
            if p:
                decls.append((p, scope[p]))
        else:
            decls.append((rng.choice(PREFIXES), rng.choice(URIS)))
    seen, clean = set(), []
    for p, u in decls:
        if p in seen:
            continue
        seen.add(p)
        clean.append((p, u))
        scope[p] = u
    prefixed = [p for p in scope if p and scope[p]]
    tag = rng.choice(["r", "e", "item", "a-b", "c.d"])
    if prefixed and rng.random() < 0.4:
        tag = rng.choice(prefixed) + ":" + tag
    attrs, names = [], set()
    for _ in range(rng.randrange(0, 4)):
        local = rng.choice(["a", "b", "id", "z", "m"])
        p = rng.choice(prefixed + ["", "", "xml"]) if prefixed else rng.choice(["", "", "xml"])
        if p == "xml":
            local = rng.choice(["lang", "space"])
        uri = scope.get(p, "") if p not in ("", "xml") else p
        if (uri, local) in names:
            continue
        names.add((uri, local))
        attrs.append(((p + ":" if p else "") + local, rng.choice(['"', "'"])))
    items = [("xmlns" + (":" + p if p else ""), u.replace("&", "&amp;"), '"') for p, u in clean]
    items += [(k, None, q) for k, q in attrs]
    rng.shuffle(items)
    out = "<" + tag
    for k, v, q in items:
        val = v if v is not None else attr_value(q)
        out += rng.choice([" ", "\n ", "  "]) + k + rng.choice(["=", " = "]) + q + val + q
    if depth > 3 or rng.random() < 0.25:
        doc.put(out + rng.choice(["/>", " />", "></" + tag + ">"]))
        return
    doc.put(out + ">")
    for _ in range(rng.randrange(0, 5)):
        r = rng.random()
        if r < 0.35:
            doc.put(escape_text(text_value()))
        elif r < 0.45:
            doc.put("<![CDATA[" + text_value().replace("]]>", "") + "]]>")
        elif r < 0.55:
            doc.put("<!--" + rng.choice(["", " c ", "a-b", "<x>&", "é"]) + "-->", comment=True)
        elif r < 0.62:
            doc.put("<?" + rng.choice(["pi", "app", "x-y"]) + rng.choice(["", " ", " data", "   data  more "]) + "?>")
        else:
            element(doc, depth + 1, scope)
    doc.put("</" + tag + ">")


def document():
    doc = Doc()
    if rng.random() < 0.5:
        doc.put(rng.choice(['<?xml version="1.0"?>', '<?xml version="1.0" encoding="UTF-8"?>',
                            "<?xml version='1.0' standalone='yes'?>"]) + rng.choice(["\n", "\r\n", ""]))
    for _ in range(rng.randrange(0, 3)):
        if rng.random() < 0.5:
            doc.put("<!--" + rng.choice([" prolog ", "", "x"]) + "-->", comment=True)
        else:
            doc.put("<?" + rng.choice(["style", "pi"]) + rng.choice(["", " a=1"]) + "?>")
        doc.put(rng.choice(["\n", " ", "\r\n", ""]))
    if rng.random() < 0.2:
        doc.put(rng.choice(["<!DOCTYPE r>", '<!DOCTYPE r SYSTEM "r.dtd">', "<!DOCTYPE r [ <!ELEMENT r ANY> <!-- ] -->]>"]) + "\n")
    element(doc, 0, {})
    for _ in range(rng.randrange(0, 3)):
        doc.put(rng.choice(["\n", " ", ""]))
        if rng.random() < 0.5:
            doc.put("<!--" + rng.choice([" epilog ", ""]) + "-->", comment=True)
        else:
            doc.put("<?" + rng.choice(["end", "pi"]) + "?>")
    doc.put(rng.choice(["\n", "", "\r\n"]))
    return "".join(doc.parts_c), "".join(doc.parts_nc)


BREAKS = [lambda d: d.replace("</", "</x", 1), lambda d: d.replace(">", " a='1' a='2'>", 1),
          lambda d: d + "<extra/>", lambda d: d.replace("<!--", "<!-- -- ", 1) if "<!--" in d else d + "<!-- -- -->",
          lambda d: d.replace(">", ">]]>", 1), lambda d: d.replace(">", ">&nbsp;", 1), lambda d: "",
          lambda d: d.replace(">", "><?xml version='1.0'?>", 1), lambda d: d + "trailing text",
          lambda d: d.replace("=\"", "=\"<", 1) if '="' in d else d[:-3]]
docs = []
for _ in range(1300):
    c, nc = document()
    if rng.random() < 0.12:
        brk = rng.choice(BREAKS)
        c, nc = brk(c), brk(nc)
    elif rng.random() < 0.06 and 'xmlns' in c:
        bad = rng.choice(BAD_URIS)
        c = c.replace('="' + next(u for u in URIS if '="' + u in c) + '"', '="' + bad + '"', 1) if any('="' + u in c for u in URIS) else c
        nc = nc.replace('="' + next(u for u in URIS if '="' + u in nc) + '"', '="' + bad + '"', 1) if any('="' + u in nc for u in URIS) else nc
    docs.append((c, nc))


def xmllint(args):
    flag, text = args
    path = os.path.join(work, f"d{rng.random()}.xml")
    open(path, "wb").write(text.encode())
    ran = subprocess.run(["xmllint", flag, path], capture_output=True)
    # Warnings (an odd xml:space value, say) leave the exit status at 0.
    return "ERR" if ran.returncode != 0 else ran.stdout.hex()


jobs = []
for c, nc in docs:
    jobs += [("--c14n", c), ("--exc-c14n", c), ("--c14n", nc), ("--exc-c14n", nc)]
with ThreadPoolExecutor(8) as pool:
    lint = list(pool.map(xmllint, jobs))
for k, (c, nc) in enumerate(docs):
    w = lint[4 * k:4 * k + 4]
    add("c14n", c.encode(), want=w[0])
    add("exc", c.encode(), want=w[1])
    add("c14n.nc", c.encode(), want=w[2] if w[0] != "ERR" else "ERR")
    add("exc.nc", c.encode(), want=w[3] if w[1] != "ERR" else "ERR")

# --- run -----------------------------------------------------------------------
idx = [i for i, c in enumerate(cases) if c[4] is not None]
answers = ask([cases[i][4] for i in idx])
for i, ans in zip(idx, answers):
    c = cases[i]
    if c[5] == "encoded":
        c[1] = bytes.fromhex(ans)
    elif c[5] == "same":
        c[3] = ans
    elif c[5] == "crc":
        crc = int(ans)
        c[3] = f"{crc}|{mask(crc)}|{crc}"
    elif c[5] == "natreq":
        if ans.startswith("ERR listen"):
            print("cannot listen on 127.0.0.1:5351:", ans); sys.exit(1)
        req_hex, result = ans.split("|", 1)
        c[3] = req_hex
        nxt = cases[i + 1]
        nxt[2] = bytes.fromhex(req_hex)
        nxt[3] = result

lines, batch, size = [], [], 0


def flush():
    global batch, size
    if not batch:
        return
    argv = []
    for c in batch:
        argv += [c[0], "x" + c[1].hex(), "x" + c[2].hex()]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=300)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    lines.extend(ran.stdout.split("\n")[:-1])
    batch, size = [], 0


for c in cases:
    batch.append(c)
    size += 2 * (len(c[1]) + len(c[2])) + 30
    if size > 400000 or len(batch) >= 800:
        flush()
flush()

# What was written here, read by klauspost.
second, where = [], []
for i, c in enumerate(cases):
    if isinstance(c[5], tuple) and c[5][0] in ("decodes", "stream"):
        got = lines[i] if i < len(lines) else ""
        if got.startswith("SNAPPY") or got == "<missing>":
            continue
        second.append({"op": "s2.dec" if c[5][0] == "decodes" else "s2.stream.dec", "hex": got})
        where.append(i)
back = dict(zip(where, ask(second)))

families, agree, differ = {}, 0, 0
only = os.environ.get("ONLY", "")
for i, c in enumerate(cases):
    op, a, b, want, req, then = c
    mine = lines[i] if i < len(lines) else "<missing>"
    if isinstance(then, tuple):
        reread = back.get(i)
        why = None if reread == then[1].hex() else f"klauspost reads it as {str(reread)[:80]} ({mine[:60]})"
    else:
        why = None if mine == want else f"want {str(want)[:240]!r}"
    fam = families.setdefault(op, [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if op.startswith(only) and (differ <= 12 or (only and fam[1] <= 12)):
            show = a[:300].decode("utf-8", "replace") if op.startswith(("c14n", "exc")) else a.hex()[:100]
            print(f"  {op} a={show!r} b={b[:80]!r}")
            print(f"    {why}")
            print(f"    grasm {mine[:240]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 12:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
if differ:
    print(f"files kept in {work}")
    sys.exit(1)
# A passing run leaves nothing behind: each one wrote 31 MB here.
shutil.rmtree(work, ignore_errors=True)
sys.exit(0)
