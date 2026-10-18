"""HTML charset sniffing, SVCB, DNSSEC and WKB, against Go modules in the
module cache, built offline.

- `htmlcharset`: documents assembled from byte-order marks, Content-Type
  values (well-formed and not), comments, doctypes, processing
  instructions, other tags whose attributes hide `>` and `charset`,
  raw-text elements holding decoys, and `<meta>` declarations in every
  form, some cut off by the 1,024-byte limit, against x/net's
  `charset.DetermineEncoding`; every label of the Encoding Standard, in
  odd case and whitespace, against x/text's `htmlindex`.
- `svcb`: records in presentation form packed by miekg/dns and by the
  package; wire records, well-formed and broken, unpacked by both; and the
  text the package writes, which miekg must pack back to the same bytes.
- `dnssec`: key tags, DS digests, NSEC3 hashes, type bitmaps and validity
  periods against miekg/dns; signatures miekg makes with random RSA,
  ECDSA and Ed25519 keys over random record sets (wildcards, duplicates,
  mixed case) that the package must accept, and refuse once tampered;
  and Ed25519 and P-256 signatures the package makes that miekg must
  accept.
- `wkb`: geometries of every type encoded by orb in both byte orders,
  with and without an EWKB SRID, decoded and re-encoded by the package;
  truncated, mis-typed and mixed-order values.
"""
import base64
import glob
import ipaddress
import json
import os
import random
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
sys.path.insert(0, os.path.join(ROOT, "scripts/lib/gomods"))
import oracle  # noqa: E402

stale = subprocess.run([sys.executable, "scripts/gen_htmlcharset.py", "--check"], cwd=ROOT, capture_output=True, text=True)
if stale.returncode != 0:
    print(stale.stdout + stale.stderr); sys.exit(1)

rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="b28b")
harness = os.path.join(work, "b28test2")
built = subprocess.run([GRASM, "build", "packages/b28test2/src/main.gr", "--native", "-o", harness],
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


cases = []   # [op, a bytes, b bytes, want or None, go request or None, check]


def add(op, a, b, want=None, req=None, check=None):
    cases.append([op, a, b, want, req, check])


# --- htmlcharset -------------------------------------------------------------
cache = subprocess.run(["go", "env", "GOMODCACHE"], capture_output=True, text=True).stdout.strip()
tables = os.path.join(cache, "golang.org/x/text@v0.43.0/encoding/htmlindex/tables.go")
LABELS = [e.split("=")[0] for e in ask([{"op": "labels", "args": [tables]}])[0].split(" ")]
BOGUS = ["utf8x", "latin-9", "iso-8859-0", "foo", "", "x-unknown", "utf—8", "koi8"]


def label():
    l = rng.choice(LABELS + BOGUS + ["utf-16", "utf-16le", "UTF-16BE", "x-user-defined", "replacement"])
    l = "".join(c.upper() if rng.random() < 0.3 else c for c in l)
    if rng.random() < 0.2:
        l = rng.choice([" ", "\t", "\n", "  "]) + l + rng.choice(["", " ", "\r\n"])
    return l


for l in LABELS:
    add("label", b"", l.encode(), req={"op": "label", "args": [l]})
for _ in range(400):
    l = label()
    add("label", b"", l.encode(), req={"op": "label", "args": [l]})


def q(v, quote=None):
    quote = quote if quote is not None else rng.choice(['"', "'", ""])
    return f"{quote}{v}{quote}" if quote or " " not in v else f'"{v}"'


def meta():
    kind = rng.random()
    lab = label().strip() or "utf-8"
    if kind < 0.35:
        return f"<{rng.choice(['meta', 'META', 'Meta'])}{rng.choice([' ', '  ', chr(10), '/'])}charset={q(lab)}{rng.choice(['', ' ', '/'])}>"
    if kind < 0.65:
        content = rng.choice([f"text/html; charset={lab}", f"text/html;charset={q(lab, rng.choice(['\"', '']))}",
                              f"charset = {lab}", f"text/html; charset", f"text/html; xcharset={lab}; charset={lab}"])
        attrs = [f"http-equiv={q(rng.choice(['Content-Type', 'content-type', 'refresh']), '\"')}", f"content={q(content, rng.choice(['\"', '\'']))}"]
        if rng.random() < 0.5:
            attrs.reverse()
        if rng.random() < 0.15:
            attrs = attrs[1:]
        return "<meta " + " ".join(attrs) + ">"
    if kind < 0.8:
        return f"<meta charset={q(lab)} charset={q(label().strip() or 'x')}>"
    if kind < 0.9:
        return f"<meta content=\"text/html; charset={lab}\" http-equiv=\"Content-Type\" charset=\"{label().strip()}\">"
    return f"<meta name=\"viewport\" content=\"width=device-width\">"


def noise():
    return rng.choice([
        "hello world ", "<!DOCTYPE html>", "<html lang=en>", "<head>", "<!-- <meta charset=koi8-r> -->",
        "<!-->", "<!--->", "<!-- a -- b --!>", "<?xml version=\"1.0\" encoding=\"iso-8859-2\"?>",
        "<div title=\"a > b\" data-charset=\"big5\">", "</p>", "</ meta charset=gbk>", "</>", "a < b ",
        "<title>Fake <meta charset=euc-kr></title>", "<style>p{} <meta charset=sjis></style>",
        "<textarea><meta charset=koi8-u></textarea>", "<p class=x>text</p>", "<link rel=stylesheet href=a.css>",
        "<a href='x>y'>link</a>", "<br/>", "<img src=x alt=\"<meta charset=big5>\">", "<!doctype html>",
        "<x-meta charset=gbk>", "<metas charset=gbk>", "<meta>", "<!a bogus>",
    ])


CTYPES = ["", "text/html", "text/html; charset=utf-8", "text/html; charset=\"iso-8859-2\"", "text/html;charset=KOI8-R",
          "TEXT/HTML; CHARSET=shift_jis", "text/html; charset=bogus", "charset=utf-8", "text/html; charset=a; charset=b",
          "text/html; charset=gbk; charset=gbk", "text/html; foo=bar; charset=big5", "text/html; charset=", "text/html;",
          "text/html; charset=\"euc-kr", "text/plain; charset=windows-1251 ", "application/xhtml+xml; charset=latin1",
          "text/html; charset = utf-8", "text/ html", "text/html charset=utf-8", "text/html; charset=x-sjis; q=1",
          "text/html; charset=\"utf\\-8\"", "text/html; charset=\"a\\\"b\""]
for _ in range(2500):
    parts = []
    for _ in range(rng.randrange(0, 6)):
        parts.append(noise())
    if rng.random() < 0.85:
        parts.insert(rng.randrange(len(parts) + 1), meta())
    if rng.random() < 0.3:
        parts.append(meta())
    doc = "".join(parts).encode("latin-1", "replace")
    r = rng.random()
    if r < 0.15:
        doc = b"x" * rng.randrange(980, 1030) + doc
    elif r < 0.25:
        doc = doc + "café ünïcödé".encode(rng.choice(["utf-8", "latin-1"]))
    elif r < 0.3:
        doc = doc + rb(rng.randrange(1, 6))
    if rng.random() < 0.08:
        doc = rng.choice([b"\xef\xbb\xbf", b"\xfe\xff", b"\xff\xfe"]) + doc
    if rng.random() < 0.05:
        doc = doc[:rng.randrange(len(doc) + 1)]
    ct = rng.choice(CTYPES) if rng.random() < 0.4 else ""
    add("cs", doc, ct.encode(), req={"op": "charset", "hex": doc.hex(), "args": [ct]})

# --- svcb --------------------------------------------------------------------
ALPNS = ["h2", "h3", "http/1.1", "dot", "doq", "h3-29", "a,b", "back\\slash", "sp ace", "q\"uote", "\x01\x7f", "é"]


def alpn_text(ids):
    out = []
    for s in ids:
        t = ""
        for b in s.encode("latin-1"):
            c = chr(b)
            if c == ",":
                t += "\\\\,"
            elif c == "\\":
                t += "\\\\\\\\"
            elif c in "\" ;":
                t += "\\" + c
            elif b < 0x20 or b > 0x7e:
                t += "\\%03d" % b
            else:
                t += c
        out.append(t)
    return ",".join(out)


def ipv6():
    raw = rb(16)
    if rng.random() < 0.5:
        k = rng.randrange(8)
        n = rng.randrange(1, 8 - k + 1)
        raw = raw[:2 * k] + bytes(2 * n) + raw[2 * (k + n):]
    if raw[:10] == bytes(10) and raw[10:12] == b"\xff\xff":
        raw = b"\x20" + raw[1:]
    text = str(ipaddress.IPv6Address(raw))
    if rng.random() < 0.1:
        text = text.upper()
    return text


def param(key):
    if key == "alpn":
        return alpn_text(rng.sample(ALPNS, rng.randrange(1, 4)))
    if key == "port":
        return str(rng.choice([443, 853, 8443, 0, 65535, rng.randrange(65536)]))
    if key == "ipv4hint":
        return ",".join(".".join(str(rng.randrange(256)) for _ in range(4)) for _ in range(rng.randrange(1, 4)))
    if key == "ipv6hint":
        return ",".join(ipv6() for _ in range(rng.randrange(1, 3)))
    if key == "ech":
        return base64.b64encode(rb(rng.randrange(0, 40))).decode()
    if key == "dohpath":
        return rng.choice(["/dns-query{?dns}", "/q{?dns}", "/a\\\"b{?dns}", "/x\\065y{?dns}", "/sp\\ ace{?dns}"])
    if key.startswith("key"):
        return "".join(rng.choice(["a", "b", "\\000", "\\255", "\\\\", "z", "\\\""]) for _ in range(rng.randrange(0, 6)))
    return ""


KEYS = ["alpn", "no-default-alpn", "port", "ipv4hint", "ech", "ipv6hint", "dohpath", "ohttp"]
for _ in range(900):
    keys = rng.sample(KEYS + ["key" + str(rng.randrange(9, 65535)) for _ in range(2)], rng.randrange(0, 5))
    if keys and rng.random() < 0.3:
        mand = rng.sample([k for k in keys if k not in ("mandatory",)], rng.randrange(1, len(keys) + 1))
        keys = ["mandatory"] + keys
    else:
        mand = []
    if keys and rng.random() < 0.05:
        keys.append(keys[-1])
    params = []
    for k in keys:
        if k in ("no-default-alpn", "ohttp"):
            params.append(k)
            continue
        v = ",".join(mand) if k == "mandatory" else param(k)
        quoted = rng.random() < 0.6 or " " in v or v == ""
        params.append(f'{k}="{v}"' if quoted else f"{k}={v}")
    target = rng.choice([".", "svc.example.com.", "Foo.Example.NET.", "a\\.b.example.", "x.y"])
    prio = rng.choice([0, 1, 1, 2, 65535]) if params else rng.choice([0, 1])
    text = f"{prio} {target} " + " ".join(params)
    if rng.random() < 0.04:
        text = text.replace("port=\"", "port=\"x", 1) if "port" in text else text + " bogus=1"
    add("svcb.text", b"", text.strip().encode(), req={"op": "svcb.text", "args": [f"example.com. 300 IN HTTPS {text.strip()}"]})


def wire_param(key, value):
    return struct.pack(">HH", key, len(value)) + value


for _ in range(700):
    out = struct.pack(">H", rng.randrange(65536)) + rng.choice([b"\x00", b"\x03svc\x07example\x03com\x00", b"\x01a\x00"])
    params = []
    for key in sorted(rng.sample(range(0, 12), rng.randrange(0, 5))):
        if key == 0:
            v = b"".join(struct.pack(">H", k) for k in rng.sample(range(1, 9), rng.randrange(0, 3)))
        elif key == 1:
            v = b"".join(bytes([len(s)]) + s for s in [rng.choice([b"h2", b"h3", b"x,y", b""]) for _ in range(rng.randrange(1, 3))])
        elif key in (2, 8):
            v = b""
        elif key == 3:
            v = rb(2)
        elif key == 4:
            v = rb(4 * rng.randrange(1, 3))
        elif key == 6:
            v = rb(16 * rng.randrange(1, 3))
        else:
            v = rb(rng.randrange(0, 10))
        params.append((key, v))
    r = rng.random()
    if r < 0.08 and len(params) >= 2:
        params[0], params[1] = params[1], params[0]
    elif r < 0.12 and params:
        params.append(params[-1])
    elif r < 0.16:
        params.append((65535, b""))
    elif r < 0.2:
        params.append((3, rb(3)))
    elif r < 0.24:
        params.append((6, bytes(10) + b"\xff\xff" + rb(4)))
    elif r < 0.28:
        params.append((4, rb(5)))
    elif r < 0.31:
        params.append((1, b"\x05h2"))
    elif r < 0.34:
        params.append((2, b"x"))
    data = out + b"".join(wire_param(k, v) for k, v in params)
    if rng.random() < 0.05:
        # miekg stops quietly at the end of RDATA (dynamic update's
        # convention), reading a record with no target; a cut leaves the
        # priority and part of the name.
        data = data[:rng.randrange(3, len(data) + 1)]
    add("svcb.wire", data, b"", req={"op": "svcb.wire", "hex": data.hex()})


def problems_of(data):
    """RFC 9460 section 8, written out from the wire."""
    prio = struct.unpack(">H", data[:2])[0]
    at = 2
    while data[at]:
        at += data[at] + 1
    at += 1
    params = {}
    while at < len(data):
        k, n = struct.unpack(">HH", data[at:at + 4])
        params[k] = data[at + 4:at + 4 + n]
        at += 4 + n
    names = ["mandatory", "alpn", "no-default-alpn", "port", "ipv4hint", "ech", "ipv6hint", "dohpath", "ohttp"]
    nm = lambda k: names[k] if k < len(names) else f"key{k}"  # noqa: E731
    out = []
    if prio == 0 and params:
        out.append("an alias record carries parameters")
    if 0 in params:
        v = params[0]
        if not v:
            out.append("mandatory lists no keys")
        seen = set()
        for i in range(0, len(v) - 1, 2):
            k = struct.unpack(">H", v[i:i + 2])[0]
            if k == 0:
                out.append("mandatory names itself")
            if k in seen:
                out.append(f"mandatory names {nm(k)} twice")
            seen.add(k)
            if k not in params and k != 0:
                out.append(f"mandatory names {nm(k)}, which is missing")
    if 2 in params and 1 not in params:
        out.append("no-default-alpn without alpn")
    return ";".join(out)


# --- dnssec ------------------------------------------------------------------


def name():
    labels = ["www", "Mail", "a", "EXAMPLE", "com", "org", "x-y", "z9", "deep", "Sub"]
    return ".".join(rng.choice(labels) for _ in range(rng.randrange(1, 5))) + "."


def key_tag(flags, proto, alg, key):
    """RFC 4034 appendix B, with erratum 193 for RSA/MD5."""
    if alg == 1:
        return key[-3] * 256 + key[-2]
    rdata = struct.pack(">HBB", flags, proto, alg) + key
    total = sum(b << 8 if i % 2 == 0 else b for i, b in enumerate(rdata))
    return (total + (total >> 16 & 0xFFFF)) & 0xFFFF


for _ in range(500):
    alg = rng.choice([5, 8, 13, 14, 15])
    key = rb({13: 64, 14: 96, 15: 32}.get(alg, rng.choice([67, 131, 259])))
    flags, proto = rng.choice([256, 257, rng.randrange(65536)]), rng.choice([3, 3, rng.randrange(256)])
    add("keytag", key, f"{flags}|{proto}|{alg}".encode(), req={"op": "keytag", "args": [str(flags), str(proto), str(alg)], "hex": key.hex()})
    owner = name()
    add("ds", key, f"{flags}|{proto}|{alg}|{owner}".encode(),
        req={"op": "ds", "args": [str(flags), str(proto), str(alg), owner], "hex": key.hex()})
# miekg sums RSA/MD5 keys like the others; RFC 4034 B.1 takes two bytes of
# the modulus, which is what the package does and Python checks here.
for _ in range(120):
    key = rb(rng.choice([67, 131, 259]))
    add("keytag", key, b"257|3|1", want=str(key_tag(257, 3, 1, key)))
for _ in range(500):
    n, salt, it = name(), rb(rng.randrange(0, 9)), rng.randrange(0, 40)
    add("nsec3", salt, f"{n}|{it}".encode(), req={"op": "nsec3", "args": [n, salt.hex(), str(it)]})
for _ in range(400):
    types = sorted(set(rng.choice([1, 2, 5, 6, 15, 16, 28, 33, 43, 46, 47, 48, 50, 51, 65, 99, 256, 257, 1234, 32769, 65534])
                       for _ in range(rng.randrange(1, 8))))
    add("bitmap", b"", "|".join(map(str, types)).encode(), req={"op": "bitmap", "nums": types})
for _ in range(400):
    incep, expir = rng.randrange(1 << 32), rng.randrange(1 << 32)
    now = rng.choice([incep + rng.randrange(-1000, 1000), expir + rng.randrange(-1000, 1000), rng.randrange(1 << 33),
                      rng.randrange(1 << 32)])
    add("validity", b"", f"{incep}|{expir}|{now}".encode(), req={"op": "validity", "args": [str(incep), str(expir), str(now)]})


def record(owner):
    """A record in presentation form for Go and in wire form for Grasm."""
    t = rng.choice(["A", "AAAA", "TXT", "MX", "NS", "CNAME", "SRV"])
    ttl = rng.choice([300, 3600])
    if t == "A":
        v = ".".join(str(rng.randrange(256)) for _ in range(4))
        return f"{owner} {ttl} IN A {v}", (1, bytes(int(x) for x in v.split(".")))
    if t == "AAAA":
        raw = rb(16)
        return f"{owner} {ttl} IN AAAA {ipaddress.IPv6Address(raw)}", (28, raw)
    if t == "TXT":
        s = "".join(rng.choice("abcXYZ019 ") for _ in range(rng.randrange(0, 12)))
        return f'{owner} {ttl} IN TXT "{s}"', (16, bytes([len(s)]) + s.encode())
    target = name()
    wire = b"".join(bytes([len(l)]) + l.encode() for l in target.split(".")[:-1]) + b"\0"
    if t == "MX":
        pref = rng.randrange(65536)
        return f"{owner} {ttl} IN MX {pref} {target}", (15, struct.pack(">H", pref) + wire)
    if t == "SRV":
        p, w, port = rng.randrange(65536), rng.randrange(65536), rng.randrange(65536)
        return f"{owner} {ttl} IN SRV {p} {w} {port} {target}", (33, struct.pack(">HHH", p, w, port) + wire)
    code = 2 if t == "NS" else 5
    return f"{owner} {ttl} IN {t} {target}", (code, wire)


def rrset():
    owner = name()
    wild = rng.random() < 0.2
    signed_owner = "*." + owner if wild else owner
    expanded = "x.Y." + owner if wild else owner
    first_text, (code, first) = record(signed_owner)
    kind = first_text.split()[3]
    texts, wires = [first_text], [first]
    for _ in range(rng.randrange(0, 4)):
        while True:
            t, (c, w) = record(signed_owner)
            if t.split()[3] == kind:
                texts.append(t)
                wires.append(w)
                break
    if rng.random() < 0.2:
        texts.append(texts[0])
        wires.append(wires[0])
    ttl = first_text.split()[1]
    return texts, [(expanded, code, 1, int(ttl), w) for w in wires], ttl


signs = []
for _ in range(260):
    alg = rng.choice([8, 10, 13, 13, 14, 15, 15])
    texts, recs, ttl = rrset()
    signer = rng.choice(["example.com.", "Example.COM.", "com."])
    incep, expir = rng.randrange(1 << 31), rng.randrange(1 << 32)
    signs.append((alg, texts, recs, signer, incep, expir))
answers = ask([{"op": "sign", "args": [str(alg), signer, str(incep), str(expir), "0"] + texts}
               for alg, texts, recs, signer, incep, expir in signs])
for (alg, texts, recs, signer, incep, expir), got in zip(signs, answers):
    if got.startswith("ERR"):
        print("oracle could not sign:", got, texts); sys.exit(1)
    pub, tag, labels, origttl, sig = got.split("|")
    rr = ";".join(f"{o}/{k}/{c}/{t}/{w.hex()}" for o, k, c, t, w in recs)
    head = f"{alg}|{signer}|{incep}|{expir}|{labels}|{origttl}|{tag}|{recs[0][1]}|{rr}"
    add("verify", b"", f"{head}|{pub},{sig}".encode(), want="true")
    bad = bytearray(bytes.fromhex(sig))
    bad[rng.randrange(len(bad))] ^= 1 << rng.randrange(8)
    add("verify", b"", f"{head}|{pub},{bad.hex()}".encode(), want="false")
    o, k, c, t, w = recs[0]
    changed = ";".join(f"{o2}/{k2}/{c2}/{t2}/{(w2[:-1] + bytes([w2[-1] ^ 1]) if i == 0 and w2 else w2).hex()}"
                       for i, (o2, k2, c2, t2, w2) in enumerate(recs))
    if changed != rr:
        add("verify", b"", f"{alg}|{signer}|{incep}|{expir}|{labels}|{origttl}|{tag}|{recs[0][1]}|{changed}|{pub},{sig}".encode(),
            want="false")

# Signatures made here, checked by miekg.
def ed25519_public(seed):
    from hashlib import sha512
    p = 2 ** 255 - 19
    d = -121665 * pow(121666, p - 2, p) % p
    gy = 4 * pow(5, p - 2, p) % p
    gx2 = (gy * gy - 1) * pow(d * gy * gy + 1, p - 2, p) % p
    gx = pow(gx2, (p + 3) // 8, p)
    if (gx * gx - gx2) % p:
        gx = gx * pow(2, (p - 1) // 4, p) % p
    if gx % 2:
        gx = p - gx

    def add_(P, Q):
        (x1, y1), (x2, y2) = P, Q
        t = d * x1 * x2 * y1 * y2 % p
        return ((x1 * y2 + x2 * y1) * pow(1 + t, p - 2, p) % p, (y1 * y2 + x1 * x2) * pow(1 - t, p - 2, p) % p)

    h = sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    R, Q = (0, 1), (gx, gy)
    while a:
        if a & 1:
            R = add_(R, Q)
        Q = add_(Q, Q)
        a >>= 1
    x, y = R
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def p256_public(k):
    p = 0xFFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF
    a = -3
    G = (0x6B17D1F2E12C4247F8BCE6E563A440F277037D812DEB33A0F4A13945D898C296,
         0x4FE342E2FE1A7F9B8EE7EB4A7C0F9E162BCE33576B315ECECBB6406837BF51F5)

    def add_(P, Q):
        if P is None:
            return Q
        if Q is None:
            return P
        if P[0] == Q[0] and (P[1] + Q[1]) % p == 0:
            return None
        if P == Q:
            m = (3 * P[0] * P[0] + a) * pow(2 * P[1], p - 2, p) % p
        else:
            m = (Q[1] - P[1]) * pow(Q[0] - P[0], p - 2, p) % p
        x = (m * m - P[0] - Q[0]) % p
        return (x, (m * (P[0] - x) - P[1]) % p)

    R, Q = None, G
    while k:
        if k & 1:
            R = add_(R, Q)
        Q = add_(Q, Q)
        k >>= 1
    return R[0].to_bytes(32, "big") + R[1].to_bytes(32, "big")


P256_N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
mine = []
for _ in range(260):
    alg = rng.choice([13, 15])
    texts, recs, ttl = rrset()
    # miekg verifies only owners under the signer.
    signer = "."
    incep, expir = rng.randrange(1 << 31), rng.randrange(1 << 32)
    labels = len([l for l in recs[0][0].split(".") if l]) - (2 if recs[0][0].startswith("x.Y.") else 0)
    if texts[0].startswith("*."):
        labels = len([l for l in texts[0].split()[0].split(".") if l]) - 1
    priv = rb(32) if alg == 15 else (rng.randrange(1, P256_N)).to_bytes(32, "big")
    tag = key_tag(257, 3, alg, ed25519_public(priv) if alg == 15 else p256_public(int.from_bytes(priv, "big")))
    rr = ";".join(f"{o}/{k}/{c}/{t}/{w.hex()}" for o, k, c, t, w in recs)
    head = f"{alg}|{signer}|{incep}|{expir}|{labels}|{ttl}|{tag}|{recs[0][1]}|{rr}"
    mine.append((alg, texts, recs, signer, incep, expir, labels, ttl, priv, head))
    add("sign.ed" if alg == 15 else "sign.p256", priv, head.encode(), check=("mine", len(mine) - 1))

# --- wkb ---------------------------------------------------------------------


def num():
    return rng.choice([0.0, -0.0, 1.5, -2.25, 1e300, -1e-300, 5e-324, rng.uniform(-180, 180), float(rng.randrange(-1000, 1000))])


def pts(n):
    return [[num(), num()] for _ in range(n)]


def geom(depth=0):
    t = rng.choice(["Point", "LineString", "Polygon", "MultiPoint", "MultiLineString", "MultiPolygon"] +
                   (["GeometryCollection"] if depth < 2 else []))
    if t == "Point":
        return {"t": t, "c": pts(1)[0]}
    if t in ("LineString", "MultiPoint"):
        return {"t": t, "c": pts(rng.randrange(0, 5))}
    if t in ("Polygon", "MultiLineString"):
        return {"t": t, "c": [pts(rng.randrange(0, 5)) for _ in range(rng.randrange(0, 3))]}
    if t == "MultiPolygon":
        return {"t": t, "c": [[pts(rng.randrange(0, 4)) for _ in range(rng.randrange(0, 3))] for _ in range(rng.randrange(0, 3))]}
    return {"t": t, "g": [geom(depth + 1) for _ in range(rng.randrange(0, 3))]}


wkbs = []
for _ in range(700):
    g = json.dumps(geom())
    order = rng.choice(["big", "little"])
    srid = rng.choice([0, 0, 4326, 3857, rng.randrange(1, 1 << 31)])
    wkbs.append((g, order, srid))
encoded = ask([{"op": "ewkb.enc" if s else "wkb.enc", "args": [g, o, str(s)] if s else [g, o]} for g, o, s in wkbs])
decs = []
for (g, order, srid), hexed in zip(wkbs, encoded):
    data = bytes.fromhex(hexed)
    if srid:
        add("ewkb.dec", data, b"", req={"op": "ewkb.dec", "hex": hexed})
        add("ewkb.enc", data, f"{order}|{srid}".encode(), want=hexed)
    else:
        add("wkb.dec", data, b"", req={"op": "wkb.dec", "hex": hexed})
        add("wkb.enc", data, order.encode(), want=hexed)
        add("ewkb.dec", data, b"", req={"op": "ewkb.dec", "hex": hexed})
        if len(data) > 5 and rng.random() < 0.3:
            cut = data[:rng.randrange(len(data))]
            add("wkb.dec", cut, b"", req={"op": "wkb.dec", "hex": cut.hex()})
for _ in range(150):
    base = bytes.fromhex(ask([{"op": "wkb.enc", "args": [json.dumps(geom()), "little"]}])[0])
    r = rng.random()
    if r < 0.3:
        bad = bytes([2]) + base[1:]
    elif r < 0.6:
        bad = base[:1] + struct.pack("<I", rng.choice([0, 8, 9, 15])) + base[5:]
    else:
        # A multi-point whose member is a line string.
        bad = bytes([1]) + struct.pack("<II", 4, 1) + bytes([1]) + struct.pack("<II", 2, 0)
    add("wkb.dec", bad, b"", req={"op": "wkb.dec", "hex": bad.hex()})
# Members in the other byte order.
for _ in range(100):
    pts_ = pts(rng.randrange(1, 4))
    members = b"".join(bytes([0]) + struct.pack(">I", 1) + struct.pack(">dd", *p) if rng.random() < 0.5 else
                       bytes([1]) + struct.pack("<I", 1) + struct.pack("<dd", *p) for p in pts_)
    data = bytes([1]) + struct.pack("<II", 4, len(pts_)) + members
    add("wkb.dec", data, b"", req={"op": "wkb.dec", "hex": data.hex()})

# --- run ---------------------------------------------------------------------
go_idx = [i for i, c in enumerate(cases) if c[4] is not None]
go_ans = ask([cases[i][4] for i in go_idx])
for i, ans in zip(go_idx, go_ans):
    c = cases[i]
    op = c[0]
    if op == "svcb.text":
        c[3] = "ERR" if ans == "ERR" else ans.split("|")[0]
    elif op == "svcb.wire":
        c[3] = "ERR" if ans == "ERR" else ans.split("|")[0]
        c[5] = ("svcb", ans)
    elif op == "nsec3":
        c[3] = ans
    elif op == "bitmap":
        c[3] = ans[2:]
    else:
        c[3] = ans
for c in cases:
    if c[0] == "svcb.wire" and c[3] != "ERR" and len(c[1]) >= 3:
        add("svcb.problems", c[1], b"", want=problems_of(c[1]))

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
    if size > 200000 or len(batch) >= 1200:
        flush()
flush()

# Second round: what Grasm wrote, read by Go.
# Public keys for the signatures made here: Ed25519 from the seed, P-256 by scalar multiplication.


reqs2, idx2 = [], []
for i, c in enumerate(cases):
    mine_ans = lines[i] if i < len(lines) else "<missing>"
    if c[5] and c[5][0] == "svcb" and mine_ans != "ERR" and "|" in mine_ans:
        reqs2.append({"op": "svcb.text", "args": ["example.com. 300 IN HTTPS " + mine_ans.split("|", 1)[1]]})
        idx2.append(i)
    elif c[5] and c[5][0] == "mine":
        alg, texts, recs, signer, incep, expir, labels, ttl, priv, head = mine[c[5][1]]
        pub = ed25519_public(priv) if alg == 15 else p256_public(int.from_bytes(priv, "big"))
        tag = head.split("|")[6]
        reqs2.append({"op": "verify", "args": [str(alg), signer, str(incep), str(expir), str(labels), ttl, tag] + texts,
                      "hex": pub.hex() + "|" + (mine_ans or "00")})
        idx2.append(i)
back = dict(zip(idx2, ask(reqs2)))

families, agree, differ = {}, 0, 0
only = os.environ.get("ONLY", "")
for i, c in enumerate(cases):
    op, a, b, want, req, extra = c
    mine_ans = lines[i] if i < len(lines) else "<missing>"
    if extra and extra[0] == "svcb":
        if want == "ERR":
            why = None if mine_ans == "ERR" else f"miekg refuses; got {mine_ans[:120]}"
        else:
            hx = mine_ans.split("|")[0]
            reread = back.get(i, "ERR").split("|")[0]
            why = None if hx == want and reread == want else f"want {want}; got {mine_ans[:200]}; miekg reads the text as {reread}"
    elif extra and extra[0] == "mine":
        why = None if back.get(i) == "true" else f"miekg says {back.get(i)} to signature {mine_ans[:40]}"
    else:
        why = None if mine_ans == want else f"want {str(want)[:200]!r}"
    fam = families.setdefault(op, [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if op.startswith(only) and (differ <= 12 or (only and fam[1] <= 12)):
            print(f"  {op} a={a.hex()[:80]} b={b[:160]!r}")
            print(f"    {why}")
            print(f"    grasm {mine_ans[:200]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 17:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
