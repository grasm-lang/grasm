"""B57's packages against the references they reproduce, all CPython 3.13.

- colorsys: rgb_to_yiq, yiq_to_rgb, rgb_to_hls, hls_to_rgb, rgb_to_hsv and
  hsv_to_rgb, compared by repr, errors included.
- mimetypes: a MimeTypes() instance's guess_type, guess_file_type,
  guess_all_extensions and guess_extension (Python's built-in tables).
- urllib.robotparser: RobotFileParser.parse, can_fetch, crawl_delay,
  request_rate, site_maps and str().
- reprlib: Repr(...).repr of values ast.literal_eval reads, with every
  limit, fillvalue and indent.
- urllib.request.parse_http_list and parse_keqv_list; http.cookiejar's
  split_header_words, join_header_words, parse_ns_headers, http2time and
  iso2time (with the current year given).
- time.strptime and datetime.strptime (C locale, TZ=UTC), errors included.
"""
import math
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B57_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B57_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B57_LIB") or os.path.join(ROOT, "scripts/lib/b57")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b57")
harness = os.path.join(work, "b57test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b57test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stdout[-800:] + built.stderr[-1500:]); sys.exit(1)

cases = []   # (family, op, path, want lines)
hx = lambda t: t.encode("utf-8").hex() or "-"


def case_file(text):
    p = os.path.join(work, f"c{len(cases)}_{rng.randrange(1 << 30)}.txt")
    open(p, "w", encoding="utf-8").write(text)
    return p


def dbits(v):
    return struct.unpack("<Q", struct.pack("<d", v))[0]


JAVA = os.path.join(work, "java")
os.makedirs(JAVA)
if os.path.isdir(os.path.join(LIB, "java")):
    for f in os.listdir(os.path.join(LIB, "java")):
        if f.endswith(".java"):
            shutil.copy(os.path.join(LIB, "java", f), JAVA)
if any(f.endswith(".java") for f in os.listdir(JAVA)):
    subprocess.run(["javac", "-d", JAVA] + [os.path.join(JAVA, f) for f in os.listdir(JAVA) if f.endswith(".java")], check=True)


def rust_tool(name, src):
    out = os.path.join(work, name + ".rbin")
    subprocess.run(["rustc", "-O", src, "-o", out], check=True)
    return out


def go_tool(name, src):
    mod = os.path.join(work, name)
    os.makedirs(mod)
    shutil.copy(src, os.path.join(mod, "main.go"))
    open(os.path.join(mod, "go.mod"), "w").write(f"module {name}\n\ngo 1.25\n")
    out = os.path.join(work, name + ".bin")
    subprocess.run(["go", "build", "-o", out, "."], cwd=mod, check=True)
    return out


def family(name, op, lines, oracle_argv, chunk=500):
    want = subprocess.run(oracle_argv, input="\n".join(lines) + "\n", capture_output=True, text=True).stdout.splitlines()
    if len(want) != len(lines):
        print(f"{name}: the oracle answered {len(want)} of {len(lines)}"); sys.exit(1)
    for k in range(0, len(lines), chunk):
        cases.append((name, op, case_file("\n".join(lines[k:k + chunk]) + "\n"), want[k:k + chunk]))



# --- Python's colorsys, against CPython 3.13 ---------------------------------------------------------------------

CS_EDGES = [0.0, -0.0, 1.0, 0.5, 1 / 3, 2 / 3, 1 / 6, 5 / 6, 0.25, 0.75, 1.0 - 2 ** -53, 2 ** -1074, -2 ** -1074, 1e-17, -1e-17, 1.0 + 2 ** -52,
            0.5 - 2 ** -54, 0.5 + 2 ** -53, 1 / 6 - 2 ** -55, 1 / 6 + 2 ** -55, 2 / 3 - 2 ** -53, 2 / 3 + 2 ** -53, -1.0, 2.0, 1.5, -0.5, 255.0,
            1e300, -1e300, 4.5e15, 2 ** 62, 2 ** 63, -2 ** 63, 2 ** 64 + 2 ** 12, 1.7976931348623157e308, float("inf"), float("-inf"), float("nan")]


def cs_value(kind):
    r = rng.random()
    if r < 0.45:
        return rng.random()
    if r < 0.6:
        return rng.randrange(256) / 255
    if r < 0.7:
        return rng.randrange(101) / 100
    if r < 0.85:
        return rng.choice(CS_EDGES)
    if r < 0.93:
        return rng.uniform(-2, 3)
    return rng.choice([1, -1]) * rng.random() * 10.0 ** rng.randrange(-320, 309)


def cs_triple():
    a, b, c = cs_value(0), cs_value(1), cs_value(2)
    r = rng.random()
    if r < 0.12:
        b = a
    elif r < 0.2:
        c = a
    elif r < 0.26:
        c = b
    elif r < 0.3:
        b = c = a
    return a, b, c


def pycolorsys_line():
    op = rng.choice(["rgb_to_yiq", "yiq_to_rgb", "rgb_to_hls", "hls_to_rgb", "rgb_to_hsv", "hsv_to_rgb", "rgb_to_hls", "rgb_to_hsv", "hls_to_rgb",
                     "hsv_to_rgb"])
    a, b, c = cs_triple()
    if op == "yiq_to_rgb" and rng.random() < 0.5:
        a, b, c = rng.random(), rng.uniform(-0.6, 0.6), rng.uniform(-0.53, 0.53)
    if op in ("hls_to_rgb", "hsv_to_rgb") and rng.random() < 0.2:
        # hues at and next to the sector boundaries
        a = rng.choice([k / 6 for k in range(-12, 13)] + [k / 6 + d for k in range(-6, 7) for d in (2 ** -50, -2 ** -50)])
    if op in ("hls_to_rgb", "hsv_to_rgb") and rng.random() < 0.08:
        # zero saturation
        if op == "hls_to_rgb":
            c = rng.choice([0.0, -0.0])
        else:
            b = rng.choice([0.0, -0.0])
    if op in ("rgb_to_hls", "rgb_to_hsv", "rgb_to_yiq") and rng.random() < 0.05:
        # signed zeros: max and min keep the first of equal arguments
        a, b, c = (rng.choice([0.0, -0.0, 0.0, 1.0]) for _ in range(3))
    if op == "hls_to_rgb" and rng.random() < 0.04:
        # a hue that lands on a negative whole number, with a lightness of -0.0
        a, b, c = rng.choice([-1.0, -2.0, -3.0, 0.0, -0.0, 1.0]), rng.choice([-0.0, 0.0]), rng.choice([-2.0, -1.5, 2.0, -0.5])
    if op in ("rgb_to_hls", "rgb_to_hsv") and rng.random() < 0.08:
        # divisors that come out zero: max + min, 2 - max - min, max
        x = rng.choice([rng.random(), 0.5, 1.0, 0.25])
        a, b, c = rng.choice([(x, -x, x / 2), (1 + x, 1 - x, 1.0), (0.0, -x, -x / 3), (-0.0, -x, -x), (x, -x, -x)])
        a, b, c = rng.sample([a, b, c], 3)
    hx = lambda v: struct.pack(">d", v).hex()
    return f"pycolorsys {op} {hx(a)} {hx(b)} {hx(c)}"


family("pycolorsys", "pycolorsys", [pycolorsys_line() for _ in range(int(os.environ.get("PYCOLORSYS_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "pycolorsys/oracle.py")])


# --- Python's mimetypes, against CPython 3.13 --------------------------------------------------------------------

MT_GEN = subprocess.run(["python3.13", os.path.join(LIB, "pymimetypes/gen_tables.py")], capture_output=True, text=True).stdout
if not open(os.path.join(PKG, "pymimetypes/src/main.gr"), encoding="utf-8").read().endswith(MT_GEN):
    print("pymimetypes/src/main.gr: the generated table is stale; run pymimetypes/gen_tables.py")
    sys.exit(1)
MT_TABLE = {}
for _l in MT_GEN.splitlines():
    if _l.startswith("pub let PYMIME_"):
        _name = _l.split(":")[0].split()[-1]
        MT_TABLE[_name] = __import__("json").loads(_l.split(" = ", 1)[1])
MT_EXTS = MT_TABLE["PYMIME_TYPES"][0::2] + MT_TABLE["PYMIME_COMMON"][0::2]
MT_TYPES = sorted(set(MT_TABLE["PYMIME_TYPES"][1::2] + MT_TABLE["PYMIME_COMMON"][1::2]))


def mt_case(t):
    r = rng.random()
    if r < 0.6:
        return t
    if r < 0.8:
        return t.upper()
    if r < 0.9:
        return "".join(c.upper() if rng.random() < 0.5 else c for c in t)
    return t.replace("k", "\u212a").replace("i", rng.choice(["i", "\u0130", "I"]))


def mt_ext():
    r = rng.random()
    if r < 0.6:
        return mt_case(rng.choice(MT_EXTS))
    if r < 0.75:
        return mt_case(rng.choice([".tgz", ".taz", ".tz", ".tbz2", ".txz", ".svgz", ".TGZ"]))
    if r < 0.85:
        return mt_case(rng.choice(MT_EXTS)) + rng.choice([".gz", ".Z", ".z", ".GZ", ".bz2", ".xz", ".br", ".BR"])
    return rng.choice(["", ".", "..", ".unknown", ".\u00e9", ".tar", ".gz", ".Z", ".tar.gz", ".js.map", ".x\u212a", ".html.", ".d/x"])


def mt_name():
    stem = rng.choice(["a", "file", "", ".", "..", ".bashrc", "my.archive", "dir.d/x", "dir/", "a/b/c", "\u00e9t\u00e9", "~", "x y"])
    return rng.choice(["", "", "/", "./", "/srv/www/", "C:\\dir\\"]) + stem + mt_ext()


def mt_url():
    r = rng.random()
    name = mt_name()
    if r < 0.4:
        return name
    if r < 0.7:
        scheme = rng.choice(["http", "https", "file", "ftp", "HTTP", "s3", "c", "C", "mailto", "x-y", "1http", "h\u00e9"])
        rest = rng.choice(["//example.com/", "//h:80/", "///", "", "//[::1]/", "//[::1/", "//u@h/", "//h/a;b/", "//\u2100/"])
        tail = rng.choice(["", "", "?q=1", "#frag", "?a.txt", "#x.gif", ";type=a", ";p.png"])
        return f"{scheme}:{rest}{name.lstrip('/')}{tail}"
    if r < 0.9:
        mt = rng.choice(MT_TYPES + ["", "text", "a=b/c", "image/png;charset", "x/y=z"])
        return "data:" + mt_case(mt) + rng.choice([",", ";base64,", ";charset=utf-8,", "", ";", ",x;y", "?q,", "#a,", ";a;b,", ";x=1;base64,", "/x;y/z;w,"]) + rng.choice(["", "AAA", "a,b", "#frag"])
    return rng.choice([" http://a/b.txt", "\thttp://a/b.png", "http://a/b.t\nxt", "data:,", "data:", ":x.txt", "a:b.txt", "//host/x.css", "\x01http://a/b.js"])


def pymimetypes_line():
    r = rng.random()
    strict = rng.choice(["1", "1", "0"])
    hx = lambda t: t.encode().hex() or "-"
    if r < 0.45:
        return f"pymimetypes T {strict} {hx(mt_url())}"
    if r < 0.7:
        return f"pymimetypes F {strict} {hx(mt_name())}"
    t = mt_case(rng.choice(MT_TYPES)) if rng.random() < 0.8 else rng.choice(["", "text/unknown", "TEXT", "image/svg+xml ", "x\u212a/y"])
    return f"pymimetypes {rng.choice(['A', 'X'])} {strict} {hx(t)}"


family("pymimetypes", "pymimetypes", [pymimetypes_line() for _ in range(int(os.environ.get("PYMIMETYPES_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "pymimetypes/oracle.py")])


# --- Python's urllib.robotparser, against CPython 3.13 ------------------------------------------------------------

RP_AGENTS = ["*", "*", "Googlebot", "googlebot/2.1", "Bingbot", "figtree", "FigTree", "fig", "", "*bot", "Mozilla", "İbot", "straße", "a b",
             "Google", "bot*", "x/y", "Kiwi"]
RP_PATHS = ["/", "", "/private", "/private/", "/tmp/a.html", "/%7Ejoe", "/~joe", "*", "/a b", "/a%20b", "/é", "/%C3%A9", "/x?y=1", "/x#f", "/x;p",
            "http://example.com/abs", "//host/p", "foo", "/%", "/%zz", "/A", "/a", "/cgi-bin/", "/*.gif$", "/p%3Fq", "/p?q", "http://[::1"]
RP_DELAYS = ["1", "10", " 5 ", "1.5", "x", "", "²", "١٢", "007", "-1", "3 ", "%35", "%20"]
RP_RATES = ["1/5", "3/20", "1 / 5", "1/x", "1/5/6", "/", "²/1", "1/²", "10/", "%31/2", "0/0"]
RP_NAMES = {"ua": ["User-agent", "user-agent", "USER-AGENT", " User-Agent ", "User-Kagent"],
            "dis": ["Disallow", "disallow", "DISALLOW"], "allow": ["Allow", "allow"], "delay": ["Crawl-delay", "crawl-delay"],
            "rate": ["Request-rate", "request-rate"], "sm": ["Sitemap", "sitemap"],
            "junk": ["Host", "Noindex", "Useragent", "Disallow", "# comment", "", "  ", "\t", "Allow:/x # trailing", "garbage line", ":", "User-agent"]}


def rp_line():
    kind = rng.choice(["ua", "ua", "dis", "dis", "dis", "allow", "allow", "delay", "rate", "sm", "junk", "blank", "blank"])
    if kind == "blank":
        return rng.choice(["", "", " ", "# only a comment"])
    name = rng.choice(RP_NAMES[kind])
    if kind == "junk":
        return name
    value = rng.choice({"ua": RP_AGENTS, "dis": RP_PATHS, "allow": RP_PATHS, "delay": RP_DELAYS, "rate": RP_RATES,
                        "sm": ["http://example.com/sitemap.xml", "", "/s.xml", "%41"]}[kind])
    sep = rng.choice([": ", ":", " : ", ":\t", ": "])
    tail = rng.choice(["", "", "", " # c", "#c", "  "])
    return name + sep + value + tail


def rp_directive(kind, value):
    return rng.choice(RP_NAMES[kind]) + rng.choice([": ", ":", " : ", ":\t", ": "]) + value + rng.choice(["", "", "", " # c", "#c", "  "])


def rp_group(paths):
    """User-agent lines, then rules; the paths used go to `paths`."""
    lines = [rp_directive("ua", rng.choice(RP_AGENTS)) for _ in range(rng.choice([1, 1, 1, 2, 3]))]
    for _ in range(rng.randrange(0, 6)):
        kind = rng.choice(["dis", "dis", "dis", "dis", "allow", "allow", "delay", "rate"])
        if kind in ("dis", "allow"):
            p = rng.choice(RP_PATHS)
            while p == "http://[::1" and rng.random() < 0.8:
                p = rng.choice(RP_PATHS)
            paths.append(p)
            lines.append(rp_directive(kind, p))
        else:
            value = rng.choice(RP_DELAYS if kind == "delay" else RP_RATES)
            while "\u00b2" in value and rng.random() < 0.8:
                value = rng.choice(RP_DELAYS if kind == "delay" else RP_RATES)
            lines.append(rp_directive(kind, value))
        if rng.random() < 0.1:
            lines.append(rng.choice(["# comment", "Sitemap: http://example.com/s.xml", "Host: x", "  "]))
    return lines


def rp_text(paths):
    if rng.random() < 0.15:
        return "\n".join(rp_line() for _ in range(rng.randrange(0, 14)))
    lines = []
    for _ in range(rng.randrange(0, 5)):
        lines += rp_group(paths)
        lines.append(rng.choice(["", "", "", "# between", " "]))
    if rng.random() < 0.2:
        lines.insert(rng.randrange(len(lines) + 1), rp_line())
    eol = rng.choice(["\n", "\n", "\n", "\r\n", "\r"])
    text = eol.join(lines)
    if rng.random() < 0.05:
        text = text.replace("\n", rng.choice(["\x0b", "\x1c", "\u2028", "\x85"]), 1)
    return text


RP_UAS = ["Googlebot/2.1", "googlebot", "FigTree/0.1 Robot libwww-perl/5.04", "*", "", "Mozilla/5.0", "fig", "BINGBOT", "unknown", "\u0130BOT/1",
          "STRASSE", "x", "a b/c", "\u212aIWI"]
RP_URLS = ["http://example.com/", "/", "", "/private/x", "http://www.example.com/%7Ejoe/index.html", "/~joe/", "/a%20b", "/a b", "/\u00e9", "?q", "#f",
           "/x;p", "http://[::1", "http://example.com/tmp/a.html?x=1", "//h/private", "/private", "/A", "/cgi-bin/x", "/%", "/x?y=1", "/p?q", "foo"]


def rp_url(paths):
    if paths and rng.random() < 0.6:
        p = rng.choice(paths)
        p = p if p.startswith("/") else "/" + p
        return rng.choice(["", "http://example.com"]) + p + rng.choice(["", "", "x", "/y.html", "?q=1", "#f", "%41"])
    return rng.choice(RP_URLS)


def rp_agent():
    if rng.random() < 0.5:
        a = rng.choice(RP_AGENTS)
        return rng.choice(["", "x-", "Mega"]) + rng.choice([a, a.upper(), a.lower()]) + rng.choice(["", "/1.0", " extra", "bot"])
    return rng.choice(RP_UAS)


def pyrobotparser_line():
    hx = lambda t: t.encode().hex() or "-"
    paths = []
    text = rp_text(paths)
    pairs = " ".join(f"{hx(rp_agent())} {hx(rp_url(paths))}" for _ in range(3))
    return f"pyrobotparser {hx(text)} {pairs}"


family("pyrobotparser", "pyrobotparser", [pyrobotparser_line() for _ in range(int(os.environ.get("PYROBOTPARSER_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "pyrobotparser/oracle.py")])


# --- Python's reprlib, against CPython 3.13 ----------------------------------------------------------------------

RL_STRS = ["", "a", "abc", "it's", 'say "hi"', "both ' and \"", "back\\slash", "tab\there", "new\nline", "\x00\x7f", "été", "中文",
           "​", "ࢗ", "Ᲊ", "\U0001f600", "\U0001fbef", "x" * 40, "0123456789" * 5, "long text with ' quote and more text after it"]


def rl_str():
    if rng.random() < 0.6:
        return rng.choice(RL_STRS)
    return "".join(rng.choice(["a", "b", " ", "'", '"', "\\", "\n", "é", "ࢗ", "\U0001f600", "\x01", "z"]) for _ in range(rng.randrange(0, 45)))


def rl_scalar():
    r = rng.random()
    if r < 0.3:
        return repr(rl_str())
    if r < 0.5:
        return str(rng.choice([0, 1, -1, 7, 10 ** 12, -(10 ** 30), 2 ** 64, rng.randrange(-10 ** rng.randrange(1, 60), 10 ** rng.randrange(1, 60))]))
    if r < 0.65:
        return repr(rng.choice([0.1, -0.0, 1e300, 1e-300, 123456789.123456789, 2.5, 1e16]) * rng.choice([1, -1]))
    if r < 0.7:
        return rng.choice(["1e999", "-1e999", "1j", "(1+2j)", "(-0.5-1e300j)", "3.0j"])
    if r < 0.8:
        return rng.choice(["None", "True", "False", "..."])
    return repr(bytes(rng.randrange(256) for _ in range(rng.randrange(0, 40))))


def rl_key():
    """Dict keys: one kind per dict mostly, sometimes kinds Python cannot order at all."""
    return rng.choice([lambda: str(rng.randrange(-5, 30)), lambda: repr(rl_str()[:6]), lambda: repr((rng.randrange(3), rng.randrange(3))),
                       lambda: rng.choice(["1.5", "2.5", "-0.5", "1e999", "-1e999"])])


def rl_value(depth):
    r = rng.random()
    if depth <= 0 or r < 0.35:
        return rl_scalar()
    n = rng.choice([0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 10])
    if r < 0.55:
        return "[" + ", ".join(rl_value(depth - 1) for _ in range(n)) + "]"
    if r < 0.7:
        items = [rl_value(depth - 1) for _ in range(n)]
        return "(" + ", ".join(items) + ("," if n == 1 else "") + ")"
    if r < 0.85:
        kinds = [rl_key()] if rng.random() < 0.8 else [rl_key(), rl_key()]
        return "{" + ", ".join(f"{rng.choice(kinds)()}: {rl_value(depth - 1)}" for _ in range(n)) + "}"
    if n == 0:
        return "set()" if rng.random() < 0.5 else "{}"
    kind = rl_key()
    return "{" + ", ".join(kind() for _ in range(n)) + "}"


def pyreprlib_line():
    hx = lambda t: t.encode().hex() or "-"
    value = rl_value(rng.choice([1, 2, 3, 4, 6, 8]))
    if rng.random() < 0.02:
        # around the 4300-digit limit of int's repr (a decimal literal that long does not parse)
        huge = rng.choice(["-1" + "0" * 4299, "9" * 4300, "1" + "0" * 4299, hex(10 ** 4300), "-" + hex(10 ** 4300), hex(10 ** 4300 - 1), hex(10 ** 4301)])
        value = rng.choice([huge, f"[{huge}, {value}]", f"({value}, {huge})"])
    lim = lambda d: rng.choice([d, d, d, 0, 1, 2, 3, 4, 5, 6, 8, rng.randrange(0, 50)])
    limits = [lim(6), lim(6), lim(6), lim(4), lim(6), lim(30), lim(40), lim(30)]
    if rng.random() < 0.03:
        limits[rng.randrange(1, 5)] = -1
    if rng.random() < 0.05:
        limits[rng.choice([0, 5, 6, 7])] = rng.choice([-1, -5])
    fill = rng.choice(["...", "...", "", "…", "X", "<>", "[...]"])
    indent = rng.choice(["N", "N", "N", "i0", "i2", "i4", "i-1", "s" + hx("\t"), "s" + hx("--"), "s-", "s" + hx("·")])
    return f"pyreprlib {hx(value)} {' '.join(map(str, limits))} {hx(fill)} {indent}"


family("pyreprlib", "pyreprlib", [pyreprlib_line() for _ in range(int(os.environ.get("PYREPRLIB_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "pyreprlib/oracle.py")])


# --- Python's HTTP header helpers (urllib.request, http.cookiejar), against CPython 3.13 ------------------------

HH_ATOMS = ["a", "bc", "tok", " ", "  ", "\t", ",", ", ", ";", "; ", "=", " = ", '"', '"x,y"', '"a\\"b"', '"\\\\"', "\\", "\\,", '"un', "é",
            " ", "　", "\x0b", "\x1c", " ", "k=v", 'k="v w"', "k=", "x=1;y=2", "\n", '"a\\\nb"']


def hh_text(n=8):
    return "".join(rng.choice(HH_ATOMS) for _ in range(rng.randrange(0, n)))


HH_MONTHS = ["Jan", "Feb", "feb", "FEB", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec", "1", "12", "13", "0", "01", "1_2", "0_1",
             "_1", "1__2", "Febr", "Jxx", "x", "007"]
HH_ZONES = ["GMT", "UTC", "UT", "Z", "gmt", "EST", "+0100", "-0130", "+01:00", "+5", "-12", "+1234", "+12345", "AM", "pm", "Am5", "Amx", "xyz", "+130",
            "-1", "z", "PST"]


def hh_num(lo, hi, width):
    return str(rng.randrange(lo, hi)).zfill(rng.choice([1, width, width]))


def hh_http_date():
    r = rng.random()
    if r < 0.25:
        day = rng.choice(["Sun", "Mon", "Wed", "Fri", "Xyz", "Sat"])
        mon = rng.choice(["Jan", "Feb", "Dec", "Jxx", "jan", "Oct"])
        if rng.random() < 0.7:
            day, mon = rng.choice(["Sun", "Mon", "Wed", "Fri", "Sat"]), rng.choice(["Jan", "Feb", "Dec", "Oct", "Mar"])
            t = f"{day}, {hh_num(1, 32, 2).zfill(2)} {mon} {rng.choice(['1994', '1970', '2038', '9999', '2026'])} {hh_num(0, 25, 2).zfill(2)}:{hh_num(0, 60, 2).zfill(2)}:{hh_num(0, 62, 2).zfill(2)} GMT"
            return t + rng.choice(["", "", "", "\n"])
        t = f"{day}, {hh_num(0, 33, 2)} {mon} {rng.choice(['1994', '1969', '1970', '2038', '9999', '0001', '199'])} {hh_num(0, 26, 2)}:{hh_num(0, 61, 2)}:{hh_num(0, 63, 2)} GMT"
        return t + rng.choice(["", "", "", "\n", " ", "x"])
    prefix = rng.choice(["", "", "", "Wed, ", "Wednesday ", "wed,", "Thursday,  ", "  ", "\u3000", "Fri"])
    parts = {"day": hh_num(1, 32, 2), "sep1": rng.choice([" ", "-", "/", "  "]), "month": rng.choice(HH_MONTHS[:14] * 2 + ["1", "12", "01", "1_2", "0_1", "007"]),
             "sep2": rng.choice([" ", "-", "/", "  "]), "year": rng.choice(["1994", "94", "70", "69", "2026", "2", "0", "50", "51", "76", "77", "999"]),
             "clock": rng.choice(["", "", f" {hh_num(0, 24, 2)}:{hh_num(0, 60, 2)}", f" {hh_num(0, 24, 2)}:{hh_num(0, 60, 2)}:{hh_num(0, 60, 2)}",
                                  f":{hh_num(0, 24, 2)}:{hh_num(0, 60, 2)}"]),
             "zone": rng.choice(["", "", " " + rng.choice(HH_ZONES[:8]), rng.choice(HH_ZONES[:8])]), "paren": rng.choice(["", "", " (PST)", "(EST) "]),
             "end": rng.choice(["", "", " ", "\n"])}
    if rng.random() < 0.4:
        # one part out of the usual
        key = rng.choice(list(parts))
        parts[key] = rng.choice({"day": ["0", "00", "100", "x", "32"], "sep1": ["\t", " - ", "_"], "month": HH_MONTHS[14:], "sep2": ["\t", "_", ""],
                                 "year": ["1000", "10000", "1969", "x9", "99999"], "clock": [" 1:2", " 10:30:5", f" {hh_num(0, 30, 2)}:{hh_num(0, 70, 2)}", " 25:00"],
                                 "zone": [" " + z for z in HH_ZONES], "paren": [" (x y)", "()", " (PST"], "end": ["x", " x"]}[key])
    return prefix + "".join(parts.values())


def hh_iso_date():
    year = rng.choice(["1994", "1970", "2038", "2026", "1969", "9999", "0094", "94", "12345"])
    s1, s2 = rng.choice(["-", "", "/"]), rng.choice(["-", "", "/"])
    t = f"{year}{s1}{hh_num(1, 13, 2)}{s2}{hh_num(1, 32, 2)}" if rng.random() < 0.7 else f"{year}{s1}{hh_num(0, 14, 2)}{s2}{hh_num(0, 33, 2)}"
    if rng.random() < 0.7:
        t += rng.choice(["T", "t", " ", "-", ":", "  "]) + hh_num(0, 26, 2) + rng.choice([":", ""]) + hh_num(0, 61, 2)
        if rng.random() < 0.6:
            t += rng.choice([":", ""]) + hh_num(0, 63, 2) + rng.choice(["", "", ".5", ".", ".123"])
    if rng.random() < 0.5:
        t += rng.choice(["", " "]) + rng.choice(["Z", "z", "+01", "-0130", "+01:30", "+1", "-12:00", "+01::30", "+0100", "UTC", "+123"])
    return rng.choice(["", " ", "\t"]) + t + rng.choice(["", "", " ", "\n", "x"])


def hh_cookie():
    parts = [rng.choice(["name=value", "a=b", "x", "=v", "", " sid = 12 ", "n=\"q\"", "k=v=w"])]
    for _ in range(rng.randrange(0, 6)):
        key = rng.choice(["expires", "Expires", "EXPIRES", "path", "Path", "domain", "secure", "Secure", "version", "Version", "port", "max-age",
                          "Max-Age", "HttpOnly", "SameSite", "", "  ", "Kxpires", "comment"])
        if rng.random() < 0.25:
            parts.append(key)
            continue
        if key.lower() in ("expires",):
            v = rng.choice([hh_http_date(), '"' + hh_http_date() + '"', "", "garbage"])
        elif key.lower() == "version":
            v = rng.choice(['"1"', "1", '"', '""', "0", " 1 "])
        else:
            v = rng.choice(["/", ".example.com", "5", "", "a b", "é"])
        parts.append(key + rng.choice(["=", " = ", "="]) + v)
    return rng.choice(["; ", ";", " ;"]).join(parts)


def hh_pairs_arg():
    hx = lambda t: t.encode().hex() or "-"
    groups = []
    for _ in range(rng.randrange(0, 4)):
        pairs = []
        for _ in range(rng.randrange(0, 4)):
            name = rng.choice(["foo", "a", "", "x y", "é"])
            v = rng.choice([None, "bar", "a b", 'q"t', "b\\s", "", "abc\n", "\n", "abc\n\n", "été", "1_2", "a-b", "²"])
            pairs.append(hx(name) + ":" + ("N" if v is None else "s" + hx(v)))
        groups.append(",".join(pairs) if pairs else "~")
    return "/".join(groups) if groups else "-"


def pyhttpheaders_line():
    hx = lambda t: t.encode().hex() or "-"
    r = rng.random()
    year = rng.choice([2026, 2026, 1999, 2049, 2051, 1970, 2000, 1950, 2075, 2076])
    if r < 0.12:
        return f"pyhttpheaders L {hx(hh_text(10))}"
    if r < 0.2:
        elts = [rng.choice(["k=v", 'k="v"', 'k="', "k=", "k", "=v", "k=v=w", '"', 'a="b"c"', "a=1", "a=2", "b=\"\"", "x=\"y"]) for _ in range(rng.randrange(0, 5))]
        return "pyhttpheaders K " + " ".join(hx(e) for e in elts) if elts else "pyhttpheaders K"
    if r < 0.4:
        return "pyhttpheaders S " + " ".join(hx(hh_text(12)) for _ in range(rng.randrange(1, 3)))
    if r < 0.5:
        return f"pyhttpheaders J {hh_pairs_arg()}"
    if r < 0.65:
        return f"pyhttpheaders N {year} " + " ".join(hx(hh_cookie()) for _ in range(rng.randrange(1, 3)))
    if r < 0.85:
        return f"pyhttpheaders H {year} {hx(hh_http_date())}"
    return f"pyhttpheaders I {year} {hx(hh_iso_date())}"


family("pyhttpheaders", "pyhttpheaders", [pyhttpheaders_line() for _ in range(int(os.environ.get("PYHTTPHEADERS_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "pyhttpheaders/oracle.py")])


# --- Python's time.strptime and datetime.strptime, against CPython 3.13 (C locale, TZ=UTC) ------------------------

SP_GEN = subprocess.run(["python3.13", os.path.join(LIB, "pystrptime/gen_tables.py")], capture_output=True, text=True).stdout
if not open(os.path.join(PKG, "pystrptime/src/main.gr"), encoding="utf-8").read().endswith(SP_GEN):
    print("pystrptime/src/main.gr: the generated table is stale; run pystrptime/gen_tables.py")
    sys.exit(1)

SP_DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
SP_MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
SP_GROUP = {"Od": "d", "Om": "m", "Oy": "y", "OH": "H", "OI": "I", "OM": "M", "OS": "S", "Ow": "w"}
SP_LOCALE = {"c": ["a", "b", "d", "H", "M", "S", "Y"], "x": ["m", "d", "y"], "X": ["H", "M", "S"]}
SP_SEPS = ["-", "/", " ", ":", "T", ".", ",", "  ", "\t", "%%", "é", "k", "s", "'", " at ", "", "", "", "(", "[x]", "K", "|", "*", "+"]


def sp_case(name):
    r = rng.random()
    if r < 0.4:
        return name
    if r < 0.6:
        return name.lower()
    if r < 0.75:
        return name.upper()
    if r < 0.9:
        return "".join(c.upper() if rng.random() < 0.5 else c for c in name)
    return name.replace("s", "ſ", 1).replace("i", rng.choice(["İ", "ı", "I"]), 1).replace("k", "K", 1)


def sp_num(v, width):
    r = rng.random()
    s = str(v)
    if r < 0.6:
        return s.zfill(width)
    if r < 0.85:
        return s
    if r < 0.93 and width == 2 and v < 10:
        return " " + s
    return "".join(chr(0x660 + int(c)) if rng.random() < 0.5 else c for c in s.zfill(width))


def sp_zone():
    sign = rng.choice("+-")
    h, m, s = rng.randrange(0, 24) if rng.random() < 0.9 else rng.randrange(24, 100), rng.randrange(60), rng.randrange(60)
    frac = rng.choice(["", "", ".5", ".123456", ".1234567"])
    return rng.choice(["Z", f"{sign}{h:02d}{m:02d}", f"{sign}{h:02d}:{m:02d}", f"{sign}{h:02d}:{m:02d}:{s:02d}{frac}", f"{sign}{h:02d}{m:02d}{s:02d}{frac}",
                       f"{sign}{h:02d}:{m:02d}{s:02d}", f"{sign}{h:02d}{m:02d}:{s:02d}", "z", f"{sign}{h:02d}:{m:02d}:{s:02d}.", f"{sign}{h:1d}{m:02d}"])


def sp_value(key, when):
    y, mo, d, hh, mi, ss, wd, yd = when
    k = SP_GROUP.get(key, key)
    if key in ("Od", "Om", "Oy", "OH", "OI", "OM", "OS"):
        v = {"d": d, "m": mo, "y": y % 100, "H": hh, "I": (hh % 12) or 12, "M": mi, "S": ss}[k]
        return rng.choice([f"{v:02d}", str(v), f" {v % 10}"])
    if key == "d":
        return sp_num(d, 2)
    if key == "m":
        return sp_num(mo, 2)
    if key == "y":
        return sp_num(y % 100, 2)
    if key in ("Y", "G"):
        return sp_num(y, 4)
    if key == "H":
        return sp_num(hh, 2)
    if key == "I":
        return sp_num((hh % 12) or 12, 2)
    if key == "M":
        return sp_num(mi, 2)
    if key == "S":
        return sp_num(ss, 2)
    if key == "f":
        return str(rng.randrange(10 ** 6)).zfill(6)[:rng.randrange(1, 7)]
    if key == "p":
        return sp_case("AM" if hh < 12 else "PM")
    if key == "a":
        return sp_case(SP_DAYS[wd][:3])
    if key == "A":
        return sp_case(SP_DAYS[wd])
    if key == "b":
        return sp_case(SP_MONTHS[mo - 1][:3])
    if key == "B":
        return sp_case(SP_MONTHS[mo - 1])
    if key == "j":
        return sp_num(yd, 3)
    if key in ("U", "W", "V"):
        return sp_num(rng.choice([0, 1, 5, 26, 52, 53, rng.randrange(54)]), 2)
    if key in ("w", "Ow"):
        return str((wd + 1) % 7)
    if key == "u":
        return str(wd + 1)
    if key == "z":
        return sp_zone()
    if key == "Z":
        return sp_case(rng.choice(["UTC", "GMT", "utc", "EST", "Z"]))
    if key == "%":
        return "%"
    return " ".join(sp_value(x, when) for x in SP_LOCALE[key])


def sp_when():
    import datetime as _dt
    if rng.random() < 0.85:
        base = _dt.datetime(1, 1, 1) + _dt.timedelta(seconds=rng.randrange(0, 315537897599))
        if rng.random() < 0.5:
            y = rng.choice([1900, 1904, 1968, 1969, 2000, 2020, 2021, 2024, 9999, 1, 1970, 2038])
            if base.month == 2 and base.day == 29:
                base = base.replace(day=28)
            base = base.replace(year=y)
        return (base.year, base.month, base.day, base.hour, base.minute, base.second, base.weekday(), base.timetuple().tm_yday)
    return (rng.choice([0, 1, 1900, 2021, 2024]), rng.randrange(1, 13), rng.randrange(1, 32), rng.randrange(0, 25), rng.randrange(0, 60),
            rng.randrange(0, 62), rng.randrange(7), rng.randrange(1, 367))


def sp_format():
    keys = ["d", "f", "H", "I", "G", "j", "m", "M", "S", "U", "w", "u", "V", "y", "Y", "z", "A", "a", "B", "b", "p", "Z", "%", "Od", "Om", "Oy", "OH",
            "OI", "OM", "OS", "Ow", "W", "c", "x", "X"]
    common = [["Y", "m", "d"], ["Y", "m", "d", "H", "M", "S"], ["Y", "m", "d", "H", "M", "S", "f", "z"], ["a", "b", "d", "H", "M", "S", "Y"], ["d", "B", "Y"],
              ["I", "M", "p"], ["Y", "j"], ["Y", "U", "w"], ["Y", "W", "a"], ["G", "V", "u"], ["G", "V", "A"], ["y", "m", "d"], ["H", "M", "S", "z", "Z"],
              ["c"], ["x"], ["X"], ["x", "X"], ["m", "d"], ["d"], ["G", "V"], ["V", "u", "Y"], ["j", "G", "V", "u"], ["Y", "m", "d", "Z"], ["z"], ["Y", "%"],
              ["A", "B", "d", "y"]]
    chosen = list(rng.choice(common)) if rng.random() < 0.6 else rng.sample(keys, rng.randrange(1, 5))
    if rng.random() < 0.15:
        chosen = rng.sample(chosen, len(chosen))
    used, out = set(), []
    for k in chosen:
        names = {SP_GROUP.get(x, x) for x in (SP_LOCALE.get(k, [k]))} - {"%"}
        if names & used and rng.random() < 0.9:
            continue
        used |= names
        out.append(k)
    return out


SP_TARGETED = [("02/29", "%m/%d"), ("2 29", "%m %d"), ("Feb 29", "%b %d"), ("29 February", "%d %B"), ("02/29 10:00", "%m/%d %H:%M"), ("2 30", "%m %d"),
               ("2021-02-29", "%Y-%m-%d"), ("2020-02-29", "%Y-%m-%d"), ("12:00:60", "%H:%M:%S"), ("12:00:61", "%H:%M:%S"), ("2020 23:59:60", "%Y %H:%M:%S"),
               ("+2400", "%z"), ("-24:00", "%z"), ("+23:59:59.999999", "%z"), ("+24:00:00", "%z"), ("-23:59:59.999999", "%z"), ("+2359", "%z"),
               ("2020-01-01 +24:00", "%Y-%m-%d %z"), ("2020-01-01 -2400 UTC", "%Y-%m-%d %z %Z"), ("2020-01-01 +0000 gmt", "%Y-%m-%d %z %Z")]


def pystrptime_line():
    hx = lambda t: t.encode().hex() or "-"
    if rng.random() < 0.04:
        data, fmt = rng.choice(SP_TARGETED)
        return f"pystrptime {rng.choice(['T', 'D'])} {hx(data)} {hx(fmt)}"
    keys = sp_format()
    when = sp_when()
    seps = [rng.choice(SP_SEPS) if rng.random() < 0.7 else rng.choice(["-", " ", ":"]) for _ in range(len(keys) + 1)]
    seps[0] = seps[0] if rng.random() < 0.2 else ""
    seps[-1] = seps[-1] if rng.random() < 0.2 else ""
    fmt = seps[0] + "".join("%" + k + seps[i + 1] for i, k in enumerate(keys))
    data_seps = [s.replace("%%", "%") for s in seps]
    data = data_seps[0] + "".join(sp_value(k, when) + data_seps[i + 1] for i, k in enumerate(keys))
    r = rng.random()
    if r < 0.08:
        fmt += rng.choice(["%q", "%.", "%O", "%Oq", "% ", "%'", "%", "%E", "%O.", "%O ", "%\\", "%[", "%Ox"])
    elif r < 0.25 and data:
        i = rng.randrange(len(data) + 1)
        data = rng.choice([data[:i] + data[i + 1:], data[:i] + rng.choice(["0", "x", " ", "1", "١", "　"]) + data[i:], data + rng.choice(["x", "1", " "]),
                           data.swapcase(), data.replace("'", "ʼ"), data.replace(" ", rng.choice(["  ", "\t", "　", "\x1c"]))])
    if rng.random() < 0.05:
        fmt = fmt.replace("'", "'") + "'"
        data = data + rng.choice(["'", "ʼ", "`"])
    return f"pystrptime {rng.choice(['T', 'D'])} {hx(data)} {hx(fmt)}"


family("pystrptime", "pystrptime", [pystrptime_line() for _ in range(int(os.environ.get("PYSTRPTIME_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "pystrptime/oracle.py")])


# FAMILIES

# --- run ------------------------------------------------------------------------------------------------

HEX_ANSWERS = {"pymimetypes", "pyrobotparser", "pyreprlib", "pystrptime"}


def shown_text(fam, line):
    """A hex answer decoded for reading."""
    if fam not in HEX_ANSWERS or line == "-":
        return line
    try:
        return repr(bytes.fromhex(line).decode("utf-8"))
    except ValueError:
        return line


def finish():
    got = []
    for i in range(0, len(cases), 50):
        argv = []
        for fam, op, path, w in cases[i:i + 50]:
            argv += [op, path]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
        if ran.returncode != 0:
            print("harness failed:", ran.stderr[-2000:]); sys.exit(1)
        got.extend(ran.stdout.split("\n")[:-1])
    families, differ, shown, at = {}, 0, 0, 0
    for fam, op, path, w in cases:
        ins = open(path, encoding="utf-8").read().splitlines()
        f = families.setdefault(fam, [0, 0])
        for k, want_line in enumerate(w):
            have = got[at] if at < len(got) else "<missing>"
            at += 1
            if have == want_line:
                f[0] += 1
                continue
            f[1] += 1
            differ += 1
            if shown < 6:
                shown += 1
                print(f"  {fam} {ins[k][:200]}\n    want {shown_text(fam, want_line)[:300]}\n    got  {shown_text(fam, have)[:300]}")
    bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
    total = sum(a + d for a, d in families.values())
    if bad:
        print("disagreeing: " + " ".join(bad) + f" (SEED={SEED})")
    print("families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
    print(f"lines: {total}   agreeing: {total - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)


finish()
