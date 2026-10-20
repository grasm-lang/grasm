"""B47's packages against the references they reproduce.

- HTTP caching policy against npm's http-cache-semantics 4.2.0, run with
  the clock each case gives (and TZ=UTC: a date with no zone is UTC here).
- Unicode blocks against Java 17's Character.UnicodeBlock, the table
  first and then lookups by code point and by name.
- Hangul syllables and jamo against Python 3.13's unicodedata (Unicode
  15.1): composition, decomposition and names.
- LZ4 frames against the lz4 1.10.0 command, both ways: its frames read
  here, and frames written here read by it.
- pprof profiles against the parser Go 1.25 vendors from google/pprof,
  including profiles Go's own runtime/pprof wrote.
- HTTP Digest authentication against curl 8.7, which answers a challenge
  served from a local server.
- gob streams against Go's encoding/gob, values and types both ways.
"""
import os
import random
import shutil
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B47_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B47_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B47_LIB") or os.path.join(ROOT, "scripts/lib/b47")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b47")
NPM = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() + "/npm/node_modules"
harness = os.path.join(work, "b47test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b47test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stdout[-800:] + built.stderr[-1500:]); sys.exit(1)

cases = []   # (family, op, path, want)
hx = lambda s: s.encode().hex() or "-"


def case_file(text):
    p = os.path.join(work, f"c{len(cases)}_{rng.randrange(1 << 30)}.txt")
    open(p, "w", encoding="utf-8").write(text)
    return p


# --- HTTP caching ----------------------------------------------------------------------------------------

DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
LONGDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
BASE = 1760000000000


def http_date(ms):
    import datetime
    t = datetime.datetime.fromtimestamp(ms / 1000, datetime.timezone.utc)
    form = rng.randrange(10)
    if form < 6:
        return f"{DAYS[t.weekday()]}, {t.day:02d} {MONTHS[t.month - 1]} {t.year} {t.hour:02d}:{t.minute:02d}:{t.second:02d} GMT"
    if form == 6:
        return f"{LONGDAYS[t.weekday()]}, {t.day:02d}-{MONTHS[t.month - 1]}-{t.year % 100:02d} {t.hour:02d}:{t.minute:02d}:{t.second:02d} GMT"
    if form == 7:
        return f"{DAYS[t.weekday()]} {MONTHS[t.month - 1]} {t.day:2d} {t.hour:02d}:{t.minute:02d}:{t.second:02d} {t.year}"
    if form == 8:
        sign = rng.choice("+-")
        return f"{DAYS[t.weekday()]}, {t.day:02d} {MONTHS[t.month - 1]} {t.year} {t.hour:02d}:{t.minute:02d}:{t.second:02d} {sign}{rng.choice(['0000', '0200', '0530', '1100'])}"
    return rng.choice(["", "0", "-1", "1", "garbage", "Mon, 32 Jan 2020 00:00:00 GMT", "Mon, 31 Feb 2020 00:00:00 GMT",
                       "Thu, 01 Jan 1970 00:00:00 GMT", "Fri, 31 Dec 9999 23:59:59 GMT", "12345", "Mon, 00 Jan 2020 10:00:00 GMT",
                       "Tue, 15 Nov 1994 08:12:31 GMT, x", "Mon, 02 Jan 2006 24:00:00 GMT", "Mon, 02 Jan 2006 25:00:00 GMT",
                       "Mon, 02 Jan 2006 15:61:00 GMT", "Wed, 21 Oct 2015 07:28:00 GMT "])


def num():
    return rng.choice(["0", "1", "5", "60", "300", "3600", "86400", "999999", "-3", "1.5", "12x", "x", "", "0x10", " 7 ", "1e3"])


RES_DIRECTIVES = ["max-age={}", "s-maxage={}", "no-cache", "no-store", "private", "public", "must-revalidate", "proxy-revalidate",
                  "immutable", "stale-while-revalidate={}", "stale-if-error={}", "pre-check=0", "post-check=0", 'max-age="{}"',
                  "max-age=", "max-age=1=2", "MAX-AGE={}", "max-age = {}", '"', "no-cache=set-cookie", "x-ext=1"]
REQ_DIRECTIVES = ["no-cache", "no-store", "max-age={}", "min-fresh={}", "max-stale", "max-stale={}", "only-if-cached", "max-age="]


def directives(pool):
    parts = [rng.choice(pool).replace("{}", num()) for _ in range(rng.randrange(0, 4))]
    sep = rng.choice([", ", ",", " , "])
    return sep.join(parts)


def headers(kind, now):
    h = {}
    if kind == "res":
        if rng.random() < 0.75:
            h["cache-control"] = directives(RES_DIRECTIVES)
        for name, p, gen in [("expires", 0.35, lambda: http_date(now + rng.randrange(-86400000 * 3, 86400000 * 30))),
                             ("date", 0.5, lambda: http_date(now + rng.randrange(-86400000 * 2, 3600000))),
                             ("last-modified", 0.4, lambda: http_date(now - rng.randrange(-3600000, 86400000 * 400))),
                             ("age", 0.25, num), ("pragma", 0.15, lambda: rng.choice(["no-cache", "x no-cache", "foo"])),
                             ("vary", 0.25, lambda: rng.choice(["", "*", "accept", "Accept-Encoding, x-y", "accept , user-agent", "host"])),
                             ("set-cookie", 0.15, lambda: "a=b"),
                             ("etag", 0.4, lambda: rng.choice(['"abc"', 'W/"abc"', ' W/"x"', '"x"', "", '"abc"'])),
                             ("warning", 0.2, lambda: rng.choice(["110 x", "199 y, 299 z", " 112 a", "214 t", "1x9 u"])),
                             ("connection", 0.15, lambda: rng.choice(["x-foo, keep-alive", "close", "X-Foo", ""])),
                             ("x-foo", 0.2, lambda: "bar"), ("keep-alive", 0.1, lambda: "timeout=5"),
                             ("content-length", 0.2, lambda: str(rng.randrange(100))), ("content-type", 0.3, lambda: "text/plain")]:
            if rng.random() < p:
                h[name] = gen()
    else:
        for name, p, gen in [("host", 0.8, lambda: rng.choice(["a.test", "b.test"])), ("authorization", 0.15, lambda: rng.choice(["Basic x", ""])),
                             ("accept", 0.4, lambda: rng.choice(["text/html", "*/*"])), ("accept-encoding", 0.3, lambda: rng.choice(["gzip", "br"])),
                             ("user-agent", 0.3, lambda: rng.choice(["u1", "u2"])), ("x-y", 0.2, lambda: "1"),
                             ("cache-control", 0.35, lambda: directives(REQ_DIRECTIVES)), ("pragma", 0.1, lambda: "no-cache"),
                             ("if-none-match", 0.2, lambda: rng.choice(['"q"', 'W/"q", "r"', 'W/"w"'])),
                             ("if-modified-since", 0.15, lambda: http_date(now - 86400000)), ("if-range", 0.1, lambda: '"r"'),
                             ("if-match", 0.08, lambda: '"m"'), ("accept-ranges", 0.08, lambda: "bytes"),
                             ("connection", 0.1, lambda: "x-y"), ("te", 0.1, lambda: "trailers")]:
            if rng.random() < p:
                h[name] = gen()
    return h


def method():
    return rng.choice([None, "GET", "GET", "GET", "HEAD", "POST", "PUT", ""])


def fresh_case():
    """A response meant to be cached, asked for again: the hits, and the
    stale-while-revalidate and stale-if-error windows, which random headers
    almost never reach."""
    now0 = BASE + rng.randrange(0, 86400000 * 365)
    shared = rng.choice("10")
    lines = [f"OPT {shared} 0.1 0 -"]
    u = rng.choice(["/a", "/b"])
    req = {"host": "a.test"}
    if rng.random() < 0.3:
        req["accept-encoding"] = "gzip"
    lines.append(f"REQ {hx('GET')} {hx(u)}")
    for k, v in req.items():
        lines.append(f"RQH {hx(k)} {hx(v)}")
    lines.append(f"RES {rng.choice(['200', '200', '203', '301', '404'])}")
    life = rng.choice([1, 10, 60, 600, 3600, 86400, 200000])
    cc = [f"max-age={life}"] if rng.random() < 0.8 else []
    for d in ["stale-while-revalidate={}".format(rng.choice([5, 60, 3600])), "stale-if-error={}".format(rng.choice([5, 600])),
              "public", "immutable", "s-maxage={}".format(rng.choice([0, 30, 7200])), "must-revalidate", "private"]:
        if rng.random() < 0.2:
            cc.append(d)
    res = {"cache-control": ", ".join(cc), "date": http_date(now0 - rng.choice([0, 0, 5000, 600000])),
           "content-type": "text/plain"}
    if rng.random() < 0.4:
        res["etag"] = rng.choice(['"v1"', 'W/"v1"'])
    if rng.random() < 0.4:
        res["last-modified"] = http_date(now0 - rng.randrange(0, 86400000 * 400))
    if rng.random() < 0.3:
        res["vary"] = "accept-encoding"
    if rng.random() < 0.3:
        res["age"] = str(rng.choice([0, 3, 100]))
    if not cc and rng.random() < 0.5:
        res["expires"] = http_date(now0 + rng.randrange(-60000, 86400000 * 3))
    for k, v in res.items():
        lines.append(f"RSH {hx(k)} {hx(v)}")
    q = dict(req)
    if rng.random() < 0.25:
        q["cache-control"] = rng.choice(["max-stale", "max-stale=30", "min-fresh=10", "max-age=5", "no-cache"])
    if rng.random() < 0.15:
        q["accept-encoding"] = "br"
    lines.append(f"Q {hx('GET')} {hx(u)}")
    for k, v in q.items():
        lines.append(f"QH {hx(k)} {hx(v)}")
    r2 = rng.choice(["304", "304", "200", "500", "none"])
    lines.append(f"R2 {r2}")
    if r2 != "none":
        r2h = {}
        if rng.random() < 0.6 and "etag" in res:
            r2h["etag"] = rng.choice([res["etag"], '"v2"', 'W/"v1"'])
        if rng.random() < 0.4:
            r2h["cache-control"] = f"max-age={rng.choice([30, 600])}"
        if rng.random() < 0.3:
            r2h["content-length"] = "9"
        for k, v in r2h.items():
            lines.append(f"R2H {hx(k)} {hx(v)}")
    gap = rng.choice([0, 1000, life * 500, life * 1000, life * 1100, life * 2000, life * 1000 + 30000])
    now1 = now0 + gap
    now2 = now1 + rng.choice([0, 1000, 60000])
    lines.append(f"T {now0} {now1} {now2}")
    return "\n".join(lines) + "\n"


def http_case():
    if rng.random() < 0.5:
        return fresh_case()
    now0 = BASE + rng.randrange(0, 86400000 * 365)
    lines = [f"OPT {rng.choice('1110')} {rng.choice(['0.1', '0.1', '0.5', '0'])} {1 if rng.random() < 0.15 else 0} {rng.choice(['-', '-', '1000', '0'])}"]
    m, u = method(), rng.choice([None, "/a", "/a", "/b", ""])
    lines.append(f"REQ {'_' if m is None else hx(m)} {'_' if u is None else hx(u)}")
    for k, v in headers("req", now0).items():
        lines.append(f"RQH {hx(k)} {hx(v)}")
    lines.append(f"RES {rng.choice(['_', '200', '200', '200', '203', '206', '301', '302', '404', '500', '503', '999'])}")
    for k, v in headers("res", now0).items():
        lines.append(f"RSH {hx(k)} {hx(v)}")
    if rng.random() < 0.8:
        qm, qu = m, u
    else:
        qm, qu = method(), rng.choice([None, "/a", "/b"])
    lines.append(f"Q {'_' if qm is None else hx(qm)} {'_' if qu is None else hx(qu)}")
    for k, v in headers("req", now0).items():
        lines.append(f"QH {hx(k)} {hx(v)}")
    r2 = rng.choice(["none", "_", "304", "304", "304", "200", "500", "503"])
    lines.append(f"R2 {r2}")
    if r2 != "none":
        for k, v in headers("res", now0).items():
            lines.append(f"R2H {hx(k)} {hx(v)}")
    now1 = now0 + rng.choice([0, 1500, 30000, 3600000, 86400000 * 2, rng.randrange(0, 86400000 * 40)])
    now2 = now1 + rng.choice([0, 1000, rng.randrange(0, 86400000 * 3)])
    lines.append(f"T {now0} {now1} {now2}")
    return "\n".join(lines) + "\n"


http_paths = [case_file(http_case()) for _ in range(int(os.environ.get("HTTP_CASES", "600")))]
env = dict(os.environ, TZ="UTC")
oracle = subprocess.run(["node", os.path.join(LIB, "oracle.js"), NPM], input="\n".join(http_paths) + "\n",
                        capture_output=True, text=True, env=env).stdout.splitlines()
for p, w in zip(http_paths, oracle):
    cases.append(("httpcache", "httpcache", p, w))


# --- Unicode blocks, against Java 17 --------------------------------------------------------------------

JAVA = os.path.join(LIB, "java")
subprocess.run(["javac", "-d", JAVA, os.path.join(JAVA, "BlockQuery.java")], check=True)
want_tables = subprocess.run([sys.executable, os.path.join(LIB, "gen_blocks.py"), "--print"], capture_output=True, text=True).stdout
have = open(os.path.join(PKG, "unicodeblock/src/main.gr")).read()
for line in want_tables.splitlines():
    if line not in have:
        print("packages/unicodeblock: tables out of date with Java (regenerate with scripts/lib/b47/gen_blocks.py)")
        sys.exit(1)
names = [l.split()[0].replace("~", " ") for l in subprocess.run(
    ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", JAVA, "BlockNames"], capture_output=True, text=True).stdout.splitlines()]
starts = [int(l.split()[0], 16) for l in subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "Blocks"], capture_output=True, text=True).stdout.splitlines()]


def recase(n):
    r = rng.random()
    if r < 0.3:
        return n.lower()
    if r < 0.5:
        return "".join(c.lower() if rng.random() < 0.5 else c for c in n)
    if r < 0.6:
        return n.replace(" ", "_")
    if r < 0.7:
        return n.replace("_", " ")
    if r < 0.75:
        return " " + n
    return n


def block_lines():
    lines = []
    for _ in range(60):
        if rng.random() < 0.55:
            r = rng.random()
            if r < 0.4:
                cp = rng.choice(starts) + rng.choice([-1, 0, 1, 0])
            elif r < 0.9:
                cp = rng.randrange(0, 0x110000)
            else:
                cp = rng.choice([-1, 0x110000, 0x7FFFFFFF, 0x10FFFF, 0xE0080, 0x2FFFF])
            lines.append(f"of {'-' + format(-cp, 'x') if cp < 0 else format(cp, 'x')}")
        else:
            n = recase(rng.choice(names)) if rng.random() < 0.85 else rng.choice(
                ["", "nope", "Latin", "BASIC-LATIN", "basic latin ", "ßasic", "Greek and Coptic", "PHAGS PA", "phags-pa", "SURROGATES AREA",
                 "Latın-1 Supplement"])
            lines.append(f"name {hx(n)}")
    return "\n".join(lines) + "\n"


block_paths = [case_file(block_lines()) for _ in range(int(os.environ.get("BLOCK_CASES", "20")))]
block_out = subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "BlockQuery"], input="\n".join(block_paths) + "\n", capture_output=True, text=True).stdout.splitlines()
if len(block_out) != len(block_paths):
    print(f"Java answered {len(block_out)} of {len(block_paths)}"); sys.exit(1)
for p, w in zip(block_paths, block_out):
    cases.append(("block", "block", p, ";".join(hx(x) for x in w.split(";"))))


# --- Hangul, against Python 3.13's unicodedata ------------------------------------------------------------

want_compat = subprocess.run(["python3.13", os.path.join(LIB, "gen_hangul.py")], capture_output=True, text=True).stdout.strip()
if want_compat not in open(os.path.join(PKG, "hangul/src/main.gr")).read():
    print("packages/hangul: compatibility table out of date (regenerate with scripts/lib/b47/gen_hangul.py)"); sys.exit(1)


def hangul_char():
    r = rng.random()
    if r < 0.35:
        return chr(rng.randrange(0xAC00, 0xD7A4))
    if r < 0.5:
        return chr(rng.randrange(0x1100, 0x1113))
    if r < 0.62:
        return chr(rng.randrange(0x1161, 0x1176))
    if r < 0.72:
        return chr(rng.randrange(0x11A8, 0x11C3))
    if r < 0.77:
        return chr(rng.choice([0x1113, 0x1176, 0x11C3, 0x1160, 0x115F, 0x11A7, 0x11FF, 0xD7B0, 0xA960]))
    if r < 0.87:
        return chr(rng.choice(list(range(0x3131, 0x318F)) + list(range(0xFFA0, 0xFFDD))))
    return rng.choice("ab 1.\n")


def syllable_name_like():
    import unicodedata as u
    if rng.random() < 0.7:
        return u.name(chr(rng.randrange(0xAC00, 0xD7A4)))
    base = u.name(chr(rng.randrange(0xAC00, 0xD7A4)))
    return rng.choice([base.lower(), base + "X", base.replace("SYLLABLE ", "SYLLABLE  "), "HANGUL SYLLABLE ", "HANGUL SYLLABLE GX",
                       base[:-1], "HANGUL SYLLABLE " + rng.choice(["GGG", "AA", "YEOL", "SSA", "GYAE", "NGA", "AN"])])


def hangul_lines():
    lines = []
    for _ in range(50):
        op = rng.choice(["nfd", "nfc", "nfc", "compat", "name", "lookup", "decomp"])
        if op in ("nfd", "nfc", "compat"):
            lines.append(f"{op} {hx(''.join(hangul_char() for _ in range(rng.randrange(0, 12))))}")
        elif op in ("name", "decomp"):
            lines.append(f"{op} {format(rng.choice([rng.randrange(0xAC00, 0xD7A4), 0xABFF, 0xD7A4, 0x1100, 0x41]), 'x')}")
        else:
            lines.append(f"lookup {hx(syllable_name_like())}")
    return "\n".join(lines) + "\n"


hangul_paths = [case_file(hangul_lines()) for _ in range(int(os.environ.get("HANGUL_CASES", "20")))]
hangul_out = subprocess.run(["python3.13", os.path.join(LIB, "hangul.py")], input="\n".join(hangul_paths) + "\n",
                            capture_output=True, text=True).stdout.splitlines()
if len(hangul_out) != len(hangul_paths):
    print(f"Python answered {len(hangul_out)} of {len(hangul_paths)}"); sys.exit(1)
for p, w in zip(hangul_paths, hangul_out):
    cases.append(("hangul", "hangul", p, w))


# --- LZ4 frames, against the lz4 tool ---------------------------------------------------------------------


def sample_data():
    kind = rng.choice(["text", "zeros", "random", "mixed", "empty", "repeat", "big"])
    if kind == "text":
        words = ["the", "lz4", "frame", "block", "match", "offset", "literal", "\n", "  ", "0123456789"]
        return " ".join(rng.choice(words) for _ in range(rng.randrange(1, 3000))).encode()
    if kind == "zeros":
        return bytes(rng.choice([1, 15, 16, 300, 70000, 300000]))
    if kind == "random":
        return os.urandom(rng.choice([1, 5, 12, 13, 100, 5000, 70000]))
    if kind == "mixed":
        return b"".join(os.urandom(rng.randrange(1, 50)) * rng.randrange(1, 40) for _ in range(rng.randrange(1, 60)))
    if kind == "repeat":
        return (os.urandom(rng.randrange(1, 9)) * rng.randrange(1, 20000))[:rng.randrange(1, 140000)]
    if kind == "big":
        return (b"abcdefgh" * 40000 + os.urandom(3000)) * rng.randrange(1, 3)
    return b""


def lz4_file(data, args):
    src = os.path.join(work, f"lz{rng.randrange(1 << 40)}")
    open(src, "wb").write(data)
    r = subprocess.run(["lz4", "-q", "-f", "-c"] + args + [src], capture_output=True)
    os.remove(src)
    return r.stdout


def lz4_decode(blob):
    p = os.path.join(work, f"ld{rng.randrange(1 << 40)}.lz4")
    open(p, "wb").write(blob)
    r = subprocess.run(["lz4", "-q", "-d", "-c", p], capture_output=True)
    return (r.stdout.hex() or "-") if r.returncode == 0 else "error", p


def damage(blob):
    b = bytearray(blob)
    where = rng.choice(["header", "header", "body", "end"])
    if where == "header" and len(b) > 6:
        i = rng.randrange(0, min(len(b), 7))
    elif where == "end" and len(b) > 4:
        i = rng.randrange(len(b) - 4, len(b))
    else:
        i = rng.randrange(len(b)) if b else 0
    if b:
        b[i] ^= 1 << rng.randrange(8)
    return bytes(b)


def lz4_lines():
    lines, wants = [], []
    for _ in range(12):
        data = sample_data()
        if rng.random() < 0.3:
            # This package's encoder, decoded by lz4.
            o = [rng.choice(["65536", "262144", "1048576", "4194304"]), rng.choice("01"), rng.choice("01"), rng.choice("01")]
            lines.append(f"enc {data.hex() or '-'} {' '.join(o)}")
            wants.append(("enc", data.hex() or "-"))
            continue
        args = []
        if rng.random() < 0.15:
            args = ["-l"]
        else:
            args += [rng.choice(["-B4", "-B5", "-B6", "-B7", "-B4", "-B5"])]
            if rng.random() < 0.4:
                args.append("-BD")
            if rng.random() < 0.4:
                args.append("-BX")
            if rng.random() < 0.3:
                args.append("--no-frame-crc")
            if rng.random() < 0.3:
                args.append("--content-size")
        args.append(rng.choice(["-1", "-1", "-9", "-12"]))
        blob = lz4_file(data, args)
        if rng.random() < 0.2:
            blob = blob + lz4_file(sample_data()[:5000], ["-B4"])
        if rng.random() < 0.1:
            blob = (0x184D2A53).to_bytes(4, "little") + (5).to_bytes(4, "little") + b"skip!" + blob
        checked = "-l" not in args and ("--no-frame-crc" not in args or "-BX" in args)
        if rng.random() < 0.25 and checked:
            blob = damage(blob)
        if rng.random() < 0.05:
            blob = blob + rng.choice([b"\x00", b"xyz", b"\x01\x02\x03\x04\x05"])
        want, path = lz4_decode(blob)
        lines.append(f"dec {path}")
        wants.append(("dec", want))
    return lines, wants


def lz4_fixed():
    """The stream shapes where what follows a frame decides acceptance, every run."""
    text = b"hello hello hello hello hello"
    f, l = lz4_file(text, ["-1"]), lz4_file(text, ["-l"])
    skip0 = (0x184D2A50).to_bytes(4, "little") + bytes(4)
    # A skippable frame that claims more bytes than there are: the end of the
    # stream before any standard frame, an error after one (seed 948121430).
    over = (0x184D2A50).to_bytes(4, "little") + (100).to_bytes(4, "little") + b"ab"
    shapes = [over, over + f, l + over, f + over, over + over, skip0 + f + over,
              b"", b"xyz", f + b"\x00", f + b"\x01\x02\x03\x04\x05", l + b"xyz", l + b"\x01\x02\x03\x04\x05", l + f, l + l, f + l,
              skip0, skip0 + f, l + bytes(4), l + skip0, f + skip0, f + skip0[:6], f + f, f + (0x184D2A5F).to_bytes(4, "little") + bytes([5, 0, 0, 0]) + b"ab"]
    lines, wants = [], []
    for blob in shapes:
        want, path = lz4_decode(blob)
        lines.append(f"dec {path}")
        wants.append(("dec", want))
    return lines, wants


lz4_cases = []
lines, wants = lz4_fixed()
lz4_cases.append((case_file("\n".join(lines) + "\n"), wants))
for _ in range(int(os.environ.get("LZ4_CASES", "10"))):
    lines, wants = lz4_lines()
    p = case_file("\n".join(lines) + "\n")
    lz4_cases.append((p, wants))


# --- pprof profiles, against the parser Go vendors ---------------------------------------------------------

import gzip as _gzip

GOROOT = subprocess.run(["go", "env", "GOROOT"], capture_output=True, text=True).stdout.strip()
pp_mod = os.path.join(work, "pporacle")
os.makedirs(os.path.join(pp_mod, "profile"))
VENDORED = os.path.join(GOROOT, "src/cmd/vendor/github.com/google/pprof/profile")
for f in os.listdir(VENDORED):
    if f.endswith(".go") and not f.endswith("_test.go"):
        shutil.copy(os.path.join(VENDORED, f), os.path.join(pp_mod, "profile", f))
shutil.copy(os.path.join(LIB, "pprof/oracle.go"), os.path.join(pp_mod, "main.go"))
open(os.path.join(pp_mod, "go.mod"), "w").write("module pporacle\n\ngo 1.25\n")
pp_oracle = os.path.join(work, "pporacle_bin")
subprocess.run(["go", "build", "-o", pp_oracle, "."], cwd=pp_mod, check=True, env=dict(os.environ, GOFLAGS="-mod=mod"))


def uv(n):
    n &= (1 << 64) - 1
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def fld(num, val, wire=None):
    if isinstance(val, (bytes, bytearray)):
        return uv(num << 3 | 2) + uv(len(val)) + bytes(val)
    return uv(num << 3 | (0 if wire is None else wire)) + uv(val)


def ints(num, xs):
    if rng.random() < 0.5:
        return fld(num, b"".join(uv(x) for x in xs)) if xs else b""
    return b"".join(fld(num, x) for x in xs)


def synthetic_profile():
    strs = [""] + [rng.choice(["cpu", "nanoseconds", "samples", "count", "alloc_space", "bytes", "main.f", "main.g", "runtime.main", "/src/a.go",
                               "lib.so", "build-1", "region", "eu", "us", "bytes", "label", "é漢字", "a b", "k"]) for _ in range(rng.randrange(1, 14))]
    S_ = lambda: rng.randrange(len(strs))
    nt = rng.choice([0, 1, 2, 2, 3])
    out = bytearray()
    for _ in range(nt):
        out += fld(1, fld(1, S_()) + fld(2, S_()))
    nf = rng.randrange(0, 5)
    fids = rng.sample(range(1, 9), nf)
    funcs = b""
    for fid in fids:
        funcs += fld(5, fld(1, fid) + fld(2, S_()) + fld(3, S_()) + fld(4, S_()) + fld(5, rng.choice([0, 1, 42, (1 << 64) - 3])))
    nm = rng.randrange(0, 3)
    mids = rng.sample(range(1, 5), nm)
    maps = b""
    for mid in mids:
        body = fld(1, mid) + fld(2, rng.randrange(1 << 48)) + fld(3, rng.choice([rng.randrange(1 << 48), (1 << 64) - 1])) + fld(4, rng.randrange(1 << 20))
        body += fld(5, S_()) + fld(6, S_())
        for k in range(7, 11):
            if rng.random() < 0.5:
                body += fld(k, rng.choice([0, 1, 2]))
        maps += fld(3, body)
    nl = rng.randrange(0, 6)
    lids = rng.sample(range(1, 12), nl)
    locs = b""
    for lid in lids:
        body = fld(1, lid) + (fld(2, rng.choice(mids + [0, 99])) if rng.random() < 0.7 else b"") + fld(3, rng.choice([0, rng.randrange(1 << 40), (1 << 64) - 16]))
        for _ in range(rng.randrange(0, 3)):
            if fids:
                body += fld(4, fld(1, rng.choice(fids)) + fld(2, rng.choice([0, 7, (1 << 64) - 1])) + (fld(3, rng.randrange(80)) if rng.random() < 0.5 else b""))
        if rng.random() < 0.2:
            body += fld(5, 1)
        locs += fld(4, body)
    samples = b""
    for _ in range(rng.randrange(0, 6)):
        body = ints(1, [rng.choice(lids) for _ in range(rng.randrange(0, 4))] if lids else [])
        body += ints(2, [rng.choice([0, 1, -5 & ((1 << 64) - 1), rng.randrange(1 << 40)]) for _ in range(nt if rng.random() < 0.92 else nt + 1)])
        for _ in range(rng.randrange(0, 4)):
            r = rng.random()
            lab = fld(1, S_())
            if r < 0.45:
                lab += fld(2, rng.randrange(1, len(strs)) if len(strs) > 1 else 0)
            elif r < 0.85:
                lab += fld(3, rng.choice([1, 300, (1 << 64) - 2]))
                if rng.random() < 0.5:
                    lab += fld(4, S_())
            elif r < 0.92:
                lab += fld(4, S_())
            body += fld(3, lab)
        samples += fld(2, body)
    order = [samples, maps, locs, funcs]
    rng.shuffle(order)
    out += b"".join(order)
    for st in strs:
        out += fld(6, st.encode())
    for k, v in [(7, S_()), (8, S_()), (9, rng.choice([0, 1760000000123456789, 1760000000000000000, -1000000123, 9223372036854775807])),
                 (10, rng.choice([0, 1, 999, 1500, 1234567, 999999999, 1000000000, 61500000000, 3723000000001, 204600000, -1500000000])),
                 (12, rng.choice([0, 10000000, 512 * 1024]))]:
        if rng.random() < 0.7:
            out += fld(k, v)
    if rng.random() < 0.7:
        out += fld(11, fld(1, S_()) + fld(2, S_()))
    if rng.random() < 0.3:
        out += ints(13, [S_() for _ in range(rng.randrange(1, 3))])
    if rng.random() < 0.5:
        out += fld(14, S_())
    if rng.random() < 0.2:
        out += fld(15, S_())
    out = bytes(out)
    r = rng.random()
    if r < 0.06:
        out += fld(6, rng.choice([b"x", b""])) if rng.random() < 0.5 else fld(9, 5) + fld(9, 6)
    elif r < 0.12:
        out = out[:rng.randrange(1, len(out))] if out else out
    elif r < 0.17:
        out = out + bytes([0x82, 0x01]) + b"\xff" * 3
    elif r < 0.21:
        out = fld(6, b"oops") + out
    elif r < 0.25:
        out = out + fld(7, 9999)
    elif r < 0.28:
        out = out + uv(1 << 3 | 3)
    elif r < 0.31:
        out = out + fld(4, fld(1, 0))
    if rng.random() < 0.4:
        out = _gzip.compress(out, mtime=0)
    return out


pp_real = os.path.join(work, "realprof")
os.makedirs(pp_real)
shutil.copy(os.path.join(LIB, "pprof/realprof.go"), os.path.join(pp_real, "main.go"))
open(os.path.join(pp_real, "go.mod"), "w").write("module gop\n\ngo 1.25\n")
subprocess.run(["go", "run", ".", os.path.join(work, "cpu.pb.gz"), os.path.join(work, "heap.pb.gz")], cwd=pp_real, capture_output=True,
               env=dict(os.environ, GOFLAGS="-mod=mod"))
pp_paths = [os.path.join(work, "cpu.pb.gz"), os.path.join(work, "heap.pb.gz")]
for _ in range(int(os.environ.get("PPROF_CASES", "300"))):
    p = os.path.join(work, f"pp{rng.randrange(1 << 40)}.pb")
    open(p, "wb").write(synthetic_profile())
    pp_paths.append(p)
pp_out = subprocess.run([pp_oracle], input="\n".join(pp_paths) + "\n", capture_output=True, text=True, env=dict(os.environ, TZ="UTC")).stdout.splitlines()
if len(pp_out) != len(pp_paths):
    print(f"the pprof oracle answered {len(pp_out)} of {len(pp_paths)}"); sys.exit(1)
for k in range(0, len(pp_paths), 25):
    chunk = list(zip(pp_paths[k:k + 25], pp_out[k:k + 25]))
    cases.append(("pprof", "pprof", case_file("\n".join(p for p, w in chunk) + "\n"), ";".join(w for p, w in chunk)))


# --- HTTP Digest, against curl -------------------------------------------------------------------------------

import http.server
import threading
import re as _re


class _DigestServer(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def answer(self):
        a = self.headers.get("Authorization")
        n = int(self.headers.get("Content-Length") or 0)
        if n:
            self.rfile.read(n)
        if a is None:
            self.send_response(401)
            # send_header writes Latin-1; these code points are the UTF-8 bytes.
            self.send_header("WWW-Authenticate", self.server.challenge.encode("utf-8").decode("latin-1"))
        else:
            self.server.seen.append(a)
            self.send_response(200)
        self.send_header("Content-Length", "0")
        self.end_headers()

    do_GET = do_POST = do_PUT = do_DELETE = do_HEAD = answer


dsrv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _DigestServer)
threading.Thread(target=dsrv.serve_forever, daemon=True).start()


def dq(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def challenge():
    parts = []
    if rng.random() < 0.9:
        parts.append("realm=" + rng.choice([dq(rng.choice(["testrealm@host.com", "r", "", 'a"b', "x\\y", "spaced realm", "é"])), "unquoted", ""]))
    if rng.random() < 0.95:
        parts.append("nonce=" + rng.choice([dq(rng.choice(["dcd98b7102dd2f0e8b11d0f600bfb0c093", "n", '"q"', "a\\b"])), "tok123"]))
    if rng.random() < 0.6:
        parts.append("qop=" + rng.choice(['"auth"', '"auth,auth-int"', '"auth-int"', "auth", '" auth"', '"auth "', '"AUTH"', '"token"', "auth,auth-int", '"auth-int,auth"']))
    if rng.random() < 0.7:
        a = rng.choice(["MD5", "MD5-sess", "SHA-256", "SHA-256-sess", "SHA-512-256", "SHA-512-256-sess", "sha-256", "md5-SESS", "FOO", "SHA-256 "])
        parts.append("algorithm=" + (dq(a) if rng.random() < 0.3 else a))
    if rng.random() < 0.3:
        parts.append("opaque=" + dq(rng.choice(["5ccc069c403ebaf9f0171e9517f40e41", 'o"p', "x\\"])))
    if rng.random() < 0.25:
        parts.append("userhash=" + rng.choice(["true", "TRUE", "false", '"true"']))
    if rng.random() < 0.15:
        parts.append(rng.choice(["stale=true", "stale=FALSE", "foo=bar", "charset=UTF-8", "domain=\"/\""]))
    rng.shuffle(parts)
    sep = rng.choice([", ", ",", " , ", ",  "])
    head = rng.choice(["Digest ", "Digest ", "digest ", "DIGEST  ", "Digest\t"])
    tail = rng.choice(["", "", ",", " ", ', unterminated="x'])
    return head + sep.join(parts) + tail


def digest_case():
    ch = challenge()
    user = rng.choice(["Mufasa", "u", "a b", 'q"u', "é", "x\\y"])
    pw = rng.choice(["Circle Of Life", "", "p:w", "pässword"])
    method = rng.choice(["GET", "GET", "POST", "PUT", "DELETE", "HEAD"])
    path = rng.choice(["/dir/index.html", "/", "/a/b?x=1&y=2", "/p%20q", "/x?a=b"])
    dsrv.challenge = ch
    dsrv.seen = []
    args = ["curl", "-s", "-o", "/dev/null", "--max-time", "10", "--digest", "-u", f"{user}:{pw}"]
    if method == "HEAD":
        args.append("-I")
    elif method == "POST":
        args += ["-d", "body=1"]
    elif method != "GET":
        args += ["-X", method]
    subprocess.run(args + [f"http://127.0.0.1:{dsrv.server_address[1]}{path}"], capture_output=True)
    # http.server reads header bytes as Latin-1; these are the bytes curl sent.
    got = dsrv.seen[0].encode("latin-1").decode("utf-8", "replace") if dsrv.seen else "none"
    m = _re.search(r'cnonce="([^"]*)"', got)
    cnonce = m.group(1) if m else ""
    # HTTP strips a header value's surrounding whitespace before anything
    # reads it; the package is given the value as it arrives.
    return f"digest {hx(ch.strip(' \t'))} {hx(user)} {hx(pw)} {hx(method)} {hx(path)} {hx(cnonce)}", got


for _ in range(int(os.environ.get("DIGEST_CASES", "8"))):
    lines, wants = [], []
    for _ in range(30):
        line, want = digest_case()
        lines.append(line)
        wants.append(hx(want))
    cases.append(("digest", "digest", case_file("\n".join(lines) + "\n"), ";".join(wants)))
dsrv.shutdown()


# --- gob streams, against Go's encoding/gob -------------------------------------------------------------------

gob_mod = os.path.join(work, "goboracle")
os.makedirs(gob_mod)
shutil.copy(os.path.join(LIB, "gob/oracle.go"), os.path.join(gob_mod, "main.go"))
open(os.path.join(gob_mod, "go.mod"), "w").write("module goboracle\n\ngo 1.25\n")
gob_bin = os.path.join(work, "goboracle_bin")
subprocess.run(["go", "build", "-o", gob_bin, "."], cwd=gob_mod, check=True, env=dict(os.environ, GOFLAGS="-mod=mod"))
gob_dir = os.path.join(work, "gobs")
os.makedirs(gob_dir)
gob_rows = subprocess.run([gob_bin, str(rng.randrange(1 << 30)), os.environ.get("GOB_CASES", "300"), gob_dir], capture_output=True, text=True,
                          check=True).stdout.splitlines()
for k in range(0, len(gob_rows), 25):
    chunk = [row.split("\t") for row in gob_rows[k:k + 25]]
    cases.append(("gob", "gob", case_file("\n".join(f"{p} {n}" for p, n, w in chunk) + "\n"), ";".join(w for p, n, w in chunk)))


# --- run ------------------------------------------------------------------------------------------------

def finish():
    answers = []
    todo = [(op, path) for fam, op, path, want in cases] + [("lz4", p) for p, w in lz4_cases]
    for i in range(0, len(todo), 100):
        argv = []
        for op, path in todo[i:i + 100]:
            argv += [op, path]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
        if ran.returncode != 0:
            print("harness failed:", ran.stderr[-2000:]); sys.exit(1)
        answers.extend(ran.stdout.split("\n")[:-1])
    lz4_answers = answers[len(cases):]
    answers = answers[:len(cases)]
    for (p, wants), got in zip(lz4_cases, lz4_answers):
        outs = got.split(";")
        bad = []
        for k, ((kind, want), have) in enumerate(zip(wants, outs)):
            if kind == "enc":
                decoded, _p = lz4_decode(bytes.fromhex(have) if have != "-" else b"")
                if decoded != want:
                    bad.append(f"line {k + 1}: lz4 -d of this encoder's frame gives {decoded[:60]}, want {want[:60]}")
            elif have != want:
                bad.append(f"line {k + 1}: decoded {have[:60]}, lz4 gives {want[:60]}")
        cases.append(("lz4", "lz4", p, "ok"))
        answers.append("ok" if not bad and len(outs) == len(wants) else " | ".join(bad)[:600])
    families, differ = {}, 0
    for (fam, op, path, want), got in zip(cases, answers):
        f = families.setdefault(fam, [0, 0])
        if got == want:
            f[0] += 1
            continue
        f[1] += 1
        differ += 1
        if differ <= 6:
            def dec(h):
                try:
                    return bytes.fromhex(h).decode(errors="replace") if h and h != "-" else h
                except ValueError:
                    return h
            print(f"  {fam} {path}")
            if fam == "lz4":
                print(f"    {got}")
                continue
            if ";" in want or ";" in got:
                ins = open(path).read().splitlines()
                for k, (a, b) in enumerate(zip(want.split(";"), got.split(";") + [""] * 99)):
                    if a != b:
                        print(f"    {ins[k][:160]}\n    want {dec(a)[:200]}\n    got  {dec(b)[:200]}")
                continue
            wl, gl = dec(want).split("\n"), dec(got).split("\n")
            for a, b in zip(wl, gl + [""] * len(wl)):
                if a != b:
                    print(f"    want {a[:300]}\n    got  {b[:300]}")
    bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
    if bad:
        print("disagreeing: " + " ".join(bad) + f" (SEED={SEED})")
    print("families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
    print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)


if len(oracle) != len(http_paths):
    print(f"oracle answered {len(oracle)} of {len(http_paths)}"); sys.exit(1)
finish()
