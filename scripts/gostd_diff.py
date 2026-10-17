"""Eight packages against Go's standard library, case for case.

`fnv` `rc4` `des` `goquote` `gotimefmt` `tabwriter` `textproto` and
`suffixarray`, through the harness `packages/gostdtest`, against a small Go
program (`scripts/lib/gostd`) that answers the same questions with
`hash/fnv`, `crypto/rc4`, `crypto/des`, `strconv`, `time`, `text/tabwriter`,
`net/textproto` and `index/suffixarray`. The suffix array itself is checked
against its definition -- every suffix, sorted -- because Go does not expose
it.

Go runs with TZ=UTC, the zone `gotimefmt.parse` assumes.
"""
import os
import random
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
rng = random.Random(20261018)

work = tempfile.mkdtemp()
harness = os.path.join(work, "gostdtest")
oracle = os.path.join(work, "gostdoracle")

if subprocess.run(["go", "version"], capture_output=True).returncode != 0:
    print("go is not installed, so nothing was checked")
    sys.exit(1)
built = subprocess.run(["go", "build", "-o", oracle, "."],
                       cwd=os.path.join(ROOT, "scripts/lib/gostd"),
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
built = subprocess.run([GRASM, "build", "packages/gostdtest/src/main.gr",
                        "--native", "-o", harness], cwd=ROOT,
                       capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

# The goquote table is generated from Go; a stale one is a failure here,
# not a disagreement found later.
stale = subprocess.run(["go", "run", "scripts/gen_goprint.go", "-check",
                        "packages/goquote/src/main.gr"], cwd=ROOT,
                       capture_output=True, text=True)
if stale.returncode != 0:
    print(stale.stdout + stale.stderr); sys.exit(1)

ENV = dict(os.environ, TZ="UTC")


def ask_go(cases):
    feed = "".join(f"{op} x{a.hex()} x{b.hex()}\n" for op, a, b in cases)
    ran = subprocess.run([oracle], input=feed, capture_output=True,
                         text=True, env=ENV)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    return ran.stdout.split("\n")[:-1]


def ask_grasm(cases):
    argv = []
    for op, a, b in cases:
        argv += [op, "x" + a.hex(), "x" + b.hex()]
    lines = []
    chunk = 3 * 1500
    for start in range(0, len(argv), chunk):
        ran = subprocess.run([harness] + argv[start:start + chunk],
                             capture_output=True, text=True)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
    return lines


def rbytes(n, alphabet=None):
    if alphabet is None:
        return bytes(rng.randrange(256) for _ in range(n))
    return bytes(rng.choice(alphabet) for _ in range(n))


def utf8(r):
    """Any code point, surrogates included, encoded the way UTF-8 would."""
    if r < 0x80:
        return bytes([r])
    if r < 0x800:
        return bytes([0xC0 | r >> 6, 0x80 | r & 63])
    if r < 0x10000:
        return bytes([0xE0 | r >> 12, 0x80 | r >> 6 & 63, 0x80 | r & 63])
    return bytes([0xF0 | r >> 18, 0x80 | r >> 12 & 63, 0x80 | r >> 6 & 63,
                  0x80 | r & 63])


go_cases = []    # (op, a, b), answered by Go
py_cases = []    # (op, a, b, expected), answered here

# --- fnv ---------------------------------------------------------------
for n in list(range(0, 64)) + [100, 255, 1000] + [rng.randrange(0, 300) for _ in range(30)]:
    data = rbytes(n)
    for op in ["fnv32", "fnv32a", "fnv64", "fnv64a", "fnv128", "fnv128a"]:
        go_cases.append((op, data, b""))
for word in [b"", b"a", b"foobar", b"\x00" * 9, b"\xff" * 33]:
    for op in ["fnv32", "fnv32a", "fnv64", "fnv64a", "fnv128", "fnv128a"]:
        go_cases.append((op, word, b""))

# --- rc4 ---------------------------------------------------------------
for klen in [0, 1, 2, 5, 7, 16, 40, 128, 255, 256, 257]:
    for dlen in [0, 1, 17, 256, 700]:
        go_cases.append(("rc4", rbytes(klen), rbytes(dlen)))

# --- des ---------------------------------------------------------------
weak = [bytes.fromhex(k) for k in ["0101010101010101", "fefefefefefefefe",
                                   "e0e0e0e0f1f1f1f1", "1f1f1f1f0e0e0e0e"]]
for _ in range(120):
    klen = rng.choice([8, 8, 24, 24, 16, 7])
    key = rbytes(klen) if rng.random() > 0.1 else rng.choice(weak)
    blocks = rng.choice([0, 1, 2, 5, 16])
    tail = rng.choice([0, 0, 0, 3])
    data = rbytes(8 * blocks + tail)
    for op in ["desecb", "desecbd"]:
        go_cases.append((op, key, data))
    for op in ["descbc", "descbcd"]:
        go_cases.append((op, key, rbytes(8) + data))
go_cases.append(("descbc", rbytes(8), rbytes(5)))

# --- goquote -----------------------------------------------------------
interesting = [0, 7, 8, 9, 10, 11, 12, 13, 27, 31, 32, 34, 39, 92, 96, 126,
               127, 128, 159, 160, 173, 0x2028, 0x2029, 0x3000, 0xFEFF,
               0xFFFD, 0xFFFE, 0xFFFF, 0x10000, 0x1F600, 0xE0001, 0x10FFFF,
               0xD800, 0xDFFF, 0x110000, -1]
runes = list(range(0, 0x3100)) + interesting
runes += [rng.randrange(0x3100, 0x110000) for _ in range(6000)]
for r in runes:
    for op in ["gqr", "gqra", "gqrg", "gpr"]:
        go_cases.append((op, str(r).encode(), b""))


def quote_input():
    parts = []
    for _ in range(rng.randrange(0, 12)):
        kind = rng.random()
        if kind < 0.35:
            parts.append(rbytes(rng.randrange(1, 4), list(range(32, 127))))
        elif kind < 0.5:
            parts.append(bytes([rng.randrange(0, 32)]))
        elif kind < 0.7:
            parts.append(utf8(rng.choice(runes[:-1] if rng.random() < 0.5 else
                                         [rng.randrange(0x80, 0x110000)])))
        elif kind < 0.85:
            parts.append(bytes([rng.randrange(0x80, 0x100)]))
        else:
            # Truncated, overlong and surrogate sequences.
            parts.append(rng.choice([b"\xe2\x82", b"\xc0\xaf", b"\xed\xa0\x80",
                                     b"\xf4\x90\x80\x80", b"\xf0\x9f",
                                     b"\xe0\x80\xaf", b"\"", b"\\", b"'"]))
    return b"".join(parts)


quoted = []
for _ in range(2500):
    s = quote_input()
    for op in ["gq", "gqa", "gqg", "gcb"]:
        go_cases.append((op, s, b""))
    quoted.append(s)

literals = [b'""', b"''", b"``", b'"a"', b"'a'", b"'ab'", b"'\\''",
            b'"\\""', b"'\"'", b'"\'"', b'"\\x41"', b'"\\xff"', b'"\\u00e9"',
            b'"\\U0001F600"', b'"\\ud800"', b'"\\U00110000"', b'"\\101"',
            b'"\\400"', b'"\\8"', b'"\\q"', b'"a\nb"', b'`a\r\nb`', b'`a`b`',
            b'"abc', b"'\xff'", b"'\xe2\x82\xac'", b'"\xff"', b"`\xff`",
            b'"\\a\\b\\f\\n\\r\\t\\v\\\\"', b"x", b"", b'"x"y', b"'\\x'",
            b"'\\u12'", b"'\\377'", b'"\\0"', b'"\\00"', b'"\\000"']
mutable = b"\"'`\\xuU0789abfnrtv\n\r \xe2\x82\xac"
for _ in range(2500):
    base = rng.choice(literals)
    s = bytearray(base)
    for _ in range(rng.randrange(0, 4)):
        at = rng.randrange(0, len(s) + 1)
        if rng.random() < 0.6 or not s:
            s[at:at] = bytes([rng.choice(mutable)])
        elif at < len(s):
            del s[at]
    literals.append(bytes(s))
for lit in literals:
    go_cases.append(("guq", lit, b""))
    go_cases.append(("gqp", lit + rng.choice([b"", b" rest", b"\"", b"'x"]), b""))

# --- gotimefmt ---------------------------------------------------------
NAMED = ["01/02 03:04:05PM '06 -0700", "Mon Jan _2 15:04:05 2006",
         "Mon Jan _2 15:04:05 MST 2006", "Mon Jan 02 15:04:05 -0700 2006",
         "02 Jan 06 15:04 MST", "02 Jan 06 15:04 -0700",
         "Monday, 02-Jan-06 15:04:05 MST", "Mon, 02 Jan 2006 15:04:05 MST",
         "Mon, 02 Jan 2006 15:04:05 -0700", "2006-01-02T15:04:05Z07:00",
         "2006-01-02T15:04:05.999999999Z07:00", "3:04PM", "Jan _2 15:04:05",
         "Jan _2 15:04:05.000", "Jan _2 15:04:05.000000",
         "Jan _2 15:04:05.000000000", "2006-01-02 15:04:05", "2006-01-02",
         "15:04:05"]
TOKENS = ["2006", "06", "_2006", "01", "1", "Jan", "January", "02", "2", "_2",
          "002", "__2", "15", "03", "3", "04", "4", "05", "5", "PM", "pm",
          "Mon", "Monday", "MST", "-0700", "-07:00", "-07", "-070000",
          "-07:00:00", "Z0700", "Z07:00", "Z07", "Z070000", "Z07:00:00",
          ".000", ".999", ",000", ",999", ".0", ".9", ".000000000000",
          "Janet", "Month", "Mondays", "T", " ", "  ", "-", ":", "/", "x", "é"]


def layout():
    return "".join(rng.choice(TOKENS) for _ in range(rng.randrange(1, 9)))


def moment():
    unix = rng.choice([0, 1, -1, 86399, 951782400, 1709634000,
                       rng.randrange(-62135596800, 253402300799)])
    nanos = rng.choice([0, 1, 999999999, 120000000, 500, rng.randrange(10**9)])
    off = rng.choice([0, 3600, -25200, 19800, 20700, -34200, 50400, -43200,
                      45, -3599, rng.randrange(-50400, 50401)])
    zone = rng.choice(["", "UTC", "PST", "CEST", "+03", "GMT"])
    return f"{unix},{nanos},{off},{zone}"


fmt_cases = []
for _ in range(5000):
    lay = rng.choice(NAMED) if rng.random() < 0.3 else layout()
    fmt_cases.append(("gtf", lay.encode(), moment().encode()))
go_cases += fmt_cases

# Parse cases need values in each layout's form: format with Go first.
formatted = ask_go(fmt_cases)
parse_cases = []
for (op, lay, _), out in zip(fmt_cases, formatted):
    value = bytes.fromhex(out)
    parse_cases.append(("gtp", lay, value))
    m = bytearray(value)
    if m:
        at = rng.randrange(len(m))
        choice = rng.random()
        if choice < 0.4:
            m[at] = ord(rng.choice("0123456789 :+-ZaAMPpm.,"))
        elif choice < 0.7:
            del m[at]
        else:
            m[at:at] = rng.choice([b"0", b" ", b"9", b"UTC", b"Z"])
    parse_cases.append(("gtp", lay, bytes(m)))
for lay, val in [("2006-01-02 MST", "2024-03-05 GMT+3"),
                 ("2006-01-02 MST", "2024-03-05 GMT-11"),
                 ("2006-01-02 MST", "2024-03-05 ChST"),
                 ("2006-01-02 MST", "2024-03-05 WITA"),
                 ("2006-01-02 MST", "2024-03-05 +05"),
                 ("2006 002", "2024 060"), ("2006 002", "2023 060"),
                 ("2006 __2", "2024 366"), ("2006 __2 01", "2024  60 03"),
                 ("15:04:05", "24:00:00"), ("3:04PM", "12:30AM"),
                 ("15:04:05", "10:20:30.123456789123"),
                 ("15:04:05.000", "10:20:30.12"), ("Jan 2", "feb 30"),
                 ("06", "68"), ("06", "69"), ("06", "-1"), ("2006", "+123"),
                 ("2006-01-02T15:04:05Z07:00", "2024-03-05T10:20:30+25:00"),
                 ("2006-01-02T15:04:05Z07:00", "2024-03-05T10:20:30+00:00"),
                 ("Mon Jan 2", "MONDAY Jan 2")]:
    parse_cases.append(("gtp", lay.encode(), val.encode()))
go_cases += parse_cases

# --- tabwriter ---------------------------------------------------------
CELL_BITS = ["a", "bb", "ccc", "", "", "héllo", "日本", "x" * 9, "\xff",
             "<b>", "&amp;", "<", "&", ">", ";", "  "]


def tab_text():
    out = []
    for _ in range(rng.randrange(1, 14)):
        for _ in range(rng.randrange(0, 5)):
            piece = rng.choice(CELL_BITS)
            if piece == "\xff":
                out.append(b"\xff" + rng.choice([b"a\tb", b"", b"xyz"]) +
                           (b"\xff" if rng.random() < 0.85 else b""))
            else:
                out.append(piece.encode())
            out.append(rng.choice([b"\t", b"\t", b"\v", b""]))
        out.append(rng.choice([b"\n", b"\n", b"\n", b"\f", b""]))
    if rng.random() < 0.2:
        out.append(b"\xe2\x82")
    return b"".join(out)


for _ in range(3000):
    cfg = [rng.randrange(0, 9), rng.randrange(0, 9), rng.randrange(0, 5),
           rng.choice([32, 46, 9, 45]), rng.randrange(0, 64)]
    go_cases.append(("tw", tab_text(), ",".join(map(str, cfg)).encode()))

# --- textproto ---------------------------------------------------------
KEY_BITS = "aAzZ-_09 :é.~"
for _ in range(1500):
    key = "".join(rng.choice(KEY_BITS) for _ in range(rng.randrange(0, 12)))
    go_cases.append(("tpkey", key.encode(), b""))
for _ in range(200):
    go_cases.append(("tptrim", rbytes(rng.randrange(0, 8), b" \t\r\nab\x0b"), b""))


def header_block():
    lines = []
    for _ in range(rng.randrange(0, 7)):
        key = rng.choice(["Content-Type", "content-type", "X-Mixed-CASE",
                          "to", "Subject", "Bad Key", "", "k\x01", "Ünï",
                          "a-b-c", "--x", "Host"])
        val = rng.choice(["text/plain", "", " spaced ", "a\x7fb", "é",
                          "multi", "\tq", "v\x00"])
        lines.append(f"{key}:{rng.choice(['', ' ', '  ', chr(9)])}{val}".encode())
        while rng.random() < 0.25:
            lines.append(rng.choice([b" cont", b"\tcont", b"  ", b" \t x"]))
        if rng.random() < 0.05:
            lines.append(b"no colon here")
    if rng.random() < 0.05:
        lines.insert(0, b" leading")
    eol = rng.choice([b"\r\n", b"\n"])
    text = eol.join(lines)
    if rng.random() < 0.85:
        text += eol + eol + rng.choice([b"", b"body", b"Next: header\r\n"])
    elif lines:
        text += eol
    return text


for _ in range(2500):
    block = header_block()
    go_cases.append(("tphdr", block, b""))
    go_cases.append(("tpcont", block, b""))


def reply():
    code = rng.choice(["250", "220", "550", "099", "+25", "abc", "25", "1000"])
    lines = []
    n = rng.randrange(1, 5)
    for i in range(n):
        c = code if rng.random() < 0.85 else rng.choice(["251", "250", "xyz"])
        sep = "-" if i < n - 1 else " "
        if rng.random() < 0.05:
            sep = rng.choice(["", "_"])
        lines.append(f"{c}{sep}{rng.choice(['OK', '', 'hello world', 'é'])}")
    eol = rng.choice(["\r\n", "\n"])
    text = eol.join(lines)
    if rng.random() < 0.9:
        text += eol
    return text.encode()


for _ in range(2500):
    r = reply()
    expect = str(rng.choice([0, 2, 25, 250, 251, 5, 55, 550, 1000, -1])).encode()
    go_cases.append(("tpresp", r, expect))
    go_cases.append(("tpcode", r, expect))

for _ in range(1500):
    body = rbytes(rng.randrange(0, 30), b"..\r\n\r\nab x")
    go_cases.append(("tpdote", body, b""))
    tail = rng.choice([b".\r\n", b".\n", b"", b"\r\n.\r\n", b".", b".\r"])
    go_cases.append(("tpdotd", body + tail + rng.choice([b"", b"after"]), b""))

# --- suffixarray -------------------------------------------------------
for _ in range(500):
    n = rng.choice([0, 1, 2, 3, 9, 10, 11, rng.randrange(0, 400)])
    alphabet = rng.choice([b"a", b"ab", b"abc", b"acgt", None])
    data = rbytes(n, alphabet)
    sa = sorted(range(n), key=lambda i: data[i:])
    lcp = [0] * n
    for k in range(1, n):
        a, b = data[sa[k - 1]:], data[sa[k]:]
        m = 0
        while m < len(a) and m < len(b) and a[m] == b[m]:
            m += 1
        lcp[k] = m
    best, at = 0, 0
    for k in range(1, n):
        if lcp[k] > best:
            best, at = lcp[k], sa[k]
    py_cases.append(("sa", data, b"", ",".join(map(str, sa))))
    py_cases.append(("lcp", data, b"", ",".join(map(str, lcp))))
    py_cases.append(("sarep", data, b"", f"{at},{best}"))
    for _ in range(3):
        if n and rng.random() < 0.7:
            i = rng.randrange(n)
            pat = data[i:i + rng.randrange(0, 6)]
        else:
            pat = rbytes(rng.randrange(0, 4), alphabet)
        go_cases.append(("salook", data, pat))

expected = ask_go(go_cases) + [c[3] for c in py_cases]
cases = go_cases + [c[:3] for c in py_cases]
got = ask_grasm(cases)


def readable(text):
    try:
        return bytes.fromhex(text.split("|")[0]).decode("utf-8", "replace")
    except Exception:
        return text


families = {}
agree = differ = shown = 0
for i, (op, a, b) in enumerate(cases):
    want = expected[i]
    mine = got[i] if i < len(got) else "<missing>"
    fam = families.setdefault(op, [0, 0])
    if mine == want:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        shown += 1
        if shown <= 12:
            print(f"  {op} {a[:60]!r} {b[:40]!r}")
            print(f"    go     {want[:100]} {readable(want)[:60]!r}")
            print(f"    grasm  {mine[:100]} {readable(mine)[:60]!r}")
bad = [f"{op}:{d}" for op, (_, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
# A family with no cases would agree with anything.
thin = [op for op, (a, d) in families.items() if a + d < 50]
if thin or len(families) < 30:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
