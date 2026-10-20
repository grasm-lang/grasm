"""B51's packages against the references they reproduce.

- Node 25's path.posix: normalize, join, resolve and relative (with
  process.cwd() set per case), dirname, basename, extname, parse, format
  and isAbsolute.
- Node 25's Buffer: Buffer.from(string, encoding), toString and
  byteLength for utf8, utf16le, latin1, ascii, base64, base64url and hex,
  and the names Node accepts for them.
- V8 14's JSON.parse, with its SyntaxError messages, positions and
  context, and the key order of the objects it builds; JSON.stringify,
  with and without indentation.
- CPython 3.13's character classes on every code point (category, the str
  predicates, re's \\w \\d \\s) and the string-level predicates; the
  gate also checks the package's generated table is current.
- CPython 3.13's textwrap: TextWrapper.wrap with every option, shorten,
  dedent and indent.
- CPython 3.13's calendar in the C locale: TextCalendar formatmonth and
  formatyear, HTMLCalendar formatmonth and formatyear, monthrange,
  monthcalendar, weekday, leapdays, isleap and timegm.
- CPython 3.13's fractions.Fraction: the string and float constructors,
  arithmetic, limit_denominator, round, float(), str, repr and format.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B51_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B51_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B51_LIB") or os.path.join(ROOT, "scripts/lib/b51")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b51")
harness = os.path.join(work, "b51test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b51test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


# --- Node's path.posix, against Node 25 -----------------------------------------------------------------------

NP_SEGMENTS = ["a", "b", "c.d", ".x", "..y", "z.", "a.b.c", ".", "..", "...", "é", "😀", "\\", " ", "a..", "..a.", ".a.", "x.y.", "", ""]


def np_path():
    t = rng.choice(["", "", "/", "//", "./", "../"]) + "/".join(rng.choice(NP_SEGMENTS) for _ in range(rng.randrange(0, 5)))
    return t + rng.choice(["", "", "/", "//"])


def np_hex(t):
    return t.encode().hex() or "-"


def np_list(n):
    items = [np_path() for _ in range(n)]
    return ",".join(np_hex(x) for x in items) if items else "~"


def nodepath_line():
    cwd = rng.choice(["/", "/home/u", "/a/b/", "/x/../y", "//srv", "/é/😀", "rel/dir", ""])
    c = np_hex(cwd)
    r = rng.random()
    if r < 0.15:
        return f"no {c} {np_hex(np_path())}"
    if r < 0.27:
        return f"jo {c} {np_list(rng.randrange(0, 4))}"
    if r < 0.42:
        return f"re {c} {np_list(rng.randrange(0, 4))}"
    if r < 0.57:
        return f"rl {c} {np_hex(np_path())} {np_hex(np_path())}"
    if r < 0.65:
        return f"dn {c} {np_hex(np_path())}"
    if r < 0.77:
        p = np_path()
        sfx = rng.choice(["~", "~", np_hex(rng.choice([".d", "c.d", "d", "x", "😀", "a", "b", ".", "/", "a/"])), np_hex(p[-3:])])
        return f"bn {c} {np_hex(p)} {sfx}"
    if r < 0.85:
        return f"ex {c} {np_hex(np_path())}"
    if r < 0.88:
        return f"ab {c} {np_hex(np_path())}"
    if r < 0.96:
        return f"pa {c} {np_hex(np_path())}"
    parts = [rng.choice(["", "/", "/r", "x"]), rng.choice(["", "/", "/d", "d/e", "/r"]), rng.choice(["", "b.c", "b"]), rng.choice(["", ".e", "e", ".."]),
             rng.choice(["", "n", ".n"])]
    return f"fo {c} {','.join(np_hex(x) for x in parts)}"


family("nodepath", "nodepath", [nodepath_line() for _ in range(int(os.environ.get("NODEPATH_LINES", "8000")))], ["node", os.path.join(LIB, "nodepath/oracle.js")])


# --- Node's Buffer, against Node 25 ----------------------------------------------------------------------------

NB_ENCODINGS = ["utf8", "utf-8", "UTF8", "ucs2", "ucs-2", "utf16le", "utf-16le", "latin1", "binary", "ascii", "base64", "base64url", "hex", "Hex",
                "BASE64", "base64", "base64", "hex", "hex", "utf8", "latin2", "utf16be", "utf32"]
NB_PIECES = ["YQ", "YWJj", "ZA==", "Zm9v", "=", "==", "-_", "+/", " ", "\n", "\t", "$", "!", ".", "ab", "12", "fF", "0x", "g", "é", "€", "😀", "\ud800",
             "\udc00", "\x00", "\x7f", "A", "z", "9", "ÿ", "Ā", "\u0159\u0157", "\u3d3d", "\u0141", "\u0161", "\u0130"]


def nb_units(s):
    out = []
    for ch in s:
        c = ord(ch)
        if c >= 0x10000:
            out += [0xD800 + ((c - 0x10000) >> 10), 0xDC00 + ((c - 0x10000) & 0x3FF)]
        else:
            out.append(c)
    return ".".join("%x" % u for u in out) or "-"


def nodebuffer_line():
    enc = rng.choice(NB_ENCODINGS)
    if rng.random() < 0.6:
        return f"fs {nb_units(''.join(rng.choice(NB_PIECES) for _ in range(rng.randrange(0, 8))))} {enc}"
    r = rng.random()
    if r < 0.5:
        b = bytes(rng.randrange(256) for _ in range(rng.randrange(0, 12)))
    else:
        b = b"".join(rng.choice([b"a", b"\xc3\xa9", b"\xe2\x82\xac", b"\xf0\x9f\x98\x80", b"\xe2\x82", b"\xf0\x9f\x98", b"\xed\xa0\x80", b"\xc0\xaf", b"\xff",
                                 b"\x80", b"\xf4\x90\x80\x80", b"\xe0\x80\x80", b"\x00", b"\x7f"]) for _ in range(rng.randrange(0, 6)))
    return f"ts {b.hex() or '-'} {enc}"


family("nodebuffer", "nodebuffer", [nodebuffer_line() for _ in range(int(os.environ.get("NODEBUFFER_LINES", "8000")))],
       ["node", os.path.join(LIB, "nodebuffer/oracle.js")])


# --- V8's JSON, against Node 25 -------------------------------------------------------------------------------

JJ_KEYS = ["a", "b", "0", "1", "10", "2", "01", "-1", "1.0", "4294967294", "4294967295", "99999999999", "", "é", "😀", "\\u0031", "\\u0030", "\\ud800",
           "\\n", "key", "Z", " 3", "3 "]
JJ_NUMBERS = ["0", "-0", "1", "-1", "12", "123456789", "1234567890", "1e21", "1e-7", "1.5", "-2.5e+3", "1E400", "-1e400", "0.1", "5e-324", "100", "1e2",
              "999999999", "-999999999", "9007199254740993"]
JJ_BAD_NUMBERS = ["01", "-", "-a", "1.", "1.e5", ".5", "1e", "1e+", "+1", "0x10", "-01", "1.5.", "00"]
JJ_STR = ["a", "é", "😀", "\\\"", "\\\\", "\\/", "\\b\\f\\n\\r\\t", "\\u0041", "\\u00e9", "\\ud83d\\ude00", "\\ud800", "\\udc00", "\\u001f", " ",
          "\ud800", "\udfff", "\x7f", " "]
JJ_BAD_STR = ["\\x", "\\u12", "\\u12G4", "\\", "\x01", "\n", "\\Ā", "\\U0041"]
JJ_SPACE = ["", "", "", " ", "\n", "\t", "\r\n  "]


def jj_value(depth=0):
    sp = lambda: rng.choice(JJ_SPACE)
    r = rng.random()
    if depth > 3 or r < 0.45:
        k = rng.random()
        if k < 0.15:
            return rng.choice(["null", "true", "false"])
        if k < 0.5:
            return rng.choice(JJ_NUMBERS) if rng.random() < 0.93 else rng.choice(JJ_BAD_NUMBERS)
        pieces = [rng.choice(JJ_STR) if rng.random() < 0.95 else rng.choice(JJ_BAD_STR) for _ in range(rng.randrange(0, 4))]
        return '"' + "".join(pieces) + '"'
    if r < 0.7:
        return "[" + sp() + ("," + sp()).join(jj_value(depth + 1) + sp() for _ in range(rng.randrange(0, 4))) + "]"
    keys = [rng.choice(JJ_KEYS) for _ in range(rng.randrange(0, 5))]
    return "{" + sp() + ("," + sp()).join('"' + k + '"' + sp() + ":" + sp() + jj_value(depth + 1) + sp() for k in keys) + "}"


def jj_doc():
    r = rng.random()
    if r < 0.04:
        return rng.choice(["", " ", "NaN", "Infinity", "undefined", "[object Object]", "-Infinity", "nul", "tru", "fals", "True", "'a'", "{}", "[]",
                           "{\"a\":1,}", "[1,]", "[1 2]", "{\"a\" 1}", "{a:1}", "\"abc", "1 2", "tr\"", "[nu\"1\"]", "{\"a\":f\"\"}", " ", "﻿1"])
    t = rng.choice(JJ_SPACE) + jj_value() + rng.choice(JJ_SPACE)
    if rng.random() < 0.15:
        i = rng.randrange(0, len(t) + 1)
        k = rng.random()
        t = t[:i] if k < 0.3 else t[:i] + t[i + 1:] if k < 0.6 else t[:i] + rng.choice([",", "]", "}", ":", '"', "x", "\x00", " 1", "-", ".", "é",
                                                                                        "Ā", "😀", "\ud800", "'", "\\"]) + t[i:]
    return t


def jjhex(t):
    out = []
    for ch in t:
        c = ord(ch)
        if c >= 0x10000:
            out += [0xD800 + ((c - 0x10000) >> 10), 0xDC00 + ((c - 0x10000) & 0x3FF)]
        else:
            out.append(c)
    return ".".join("%x" % u for u in out) or "-"


def jsjson_line():
    if rng.random() < 0.6:
        return f"pa {jjhex(jj_doc())} ~"
    space = rng.choice(["~", "n0", "n2", "n4", "n10", "n11", "n-1", "n2.5", "s" + jjhex("\t"), "s" + jjhex("abcdefghijkl"), "s-", "s" + jjhex("😀😀😀😀😀😀")])
    return f"st {jjhex(jj_doc())} {space}"


family("jsjson", "jsjson", [jsjson_line() for _ in range(int(os.environ.get("JSJSON_LINES", "8000")))], ["node", os.path.join(LIB, "jsjson/oracle.js")])


# --- Python's character classes, against CPython 3.13 ---------------------------------------------------------

PC_CHARS = ["a", "A", "z", "1", "_", " ", "\t", "\x1c", "\x85", "\xa0", "²", "½", "٣", "Ⅷ", "ǅ", "ǆ", "Ǆ", "ß", "é", "É", "Σ", "ς", "ᾈ", "ⓐ", "Ⓐ", "ﬀ", "ı",
            "İ", " ", "　", "​", "﻿", "\U0001F600", "\U0001D7CE", "̀", "-", "'", ".", "$", "\x00", "\x7f", "\ud800", "",
            "\U000E0001", "͸", "a1", "Ab", "AB", "Ab Cd", "Ab cD", "x_y", "9a", "ab-c", "ⅰ", "Ⅰ"]


def pycharclass_lines():
    # Every code point, in ranges of 0x10000: the table against Python.
    lines = [f"rg {lo:x} {lo + 0x10000:x}" for lo in range(0, 0x110000, 0x10000)]
    for _ in range(int(os.environ.get("PYCHARCLASS_LINES", "4000"))):
        s = "".join(rng.choice(PC_CHARS) for _ in range(rng.randrange(0, 5)))
        if rng.random() < 0.2:
            s = chr(rng.randrange(0x110000))
        units = []
        for ch in s:
            units.append("%x" % ord(ch))
        lines.append(f"st {'.'.join(units) or '-'}")
    return lines


family("pycharclass", "pycharclass", pycharclass_lines(), ["python3.13", os.path.join(LIB, "pycharclass/oracle.py")], chunk=200)

# The committed table is what the generator writes today.
gen = subprocess.run(["python3.13", os.path.join(LIB, "pycharclass/gen_tables.py")], capture_output=True, text=True).stdout
src = open(os.path.join(PKG, "pycharclass/src/main.gr"), encoding="utf-8").read()
if not src.endswith(gen):
    print("packages/pycharclass/src/main.gr: the generated table is stale; run scripts/lib/b51/pycharclass/gen_tables.py")
    sys.exit(1)


# --- Python's textwrap, against CPython 3.13 ------------------------------------------------------------------

TW_WORDS = ["a", "the", "goof-ball", "-b", "--", "use", "option!", "Look,", "e-mail", "1-2", "x-y-z", "well--known", "mother-in-law", "foo.", "Bar",
            "end?", 'said."', "supercalifragilistic", "anti-dis-establishment", "ab", "é-è", "日本語", "_a-b", "a_-b", "---", "-", "a-", "9a-b", "a-1",
            "x　y", " ", "it's", "Mr.", "a.b", "—", "½-c", "x-y-zz", "a-b-c-de", "q-r-st", "end.\"", "go!\x27", "ok?"]
TW_SPACE = [" ", " ", " ", "  ", "\t", "\n", "\r\n", "\x0b", "\x0c", "　", "\xa0", " \t "]


def tw_text():
    out = []
    for _ in range(rng.randrange(0, 14)):
        out.append(rng.choice(TW_WORDS))
        out.append(rng.choice(TW_SPACE))
    t = "".join(out)
    return t[:-1] if t and rng.random() < 0.5 else t


def tw_opts():
    h = lambda s: s.encode().hex() or "-"
    o = tw_opts_raw(h)
    f = o.split("/")
    # CPython can loop forever when an indent fills the width (it cuts a
    # word to an empty chunk, or drops a no-break space, and starts over);
    # the oracle cannot answer those, so the width is kept wider.
    widest = max(len(bytes.fromhex(x).decode()) if x != "-" else 0 for x in f[1:3])
    if int(f[0]) - widest < 1 and int(f[0]) > 0:
        f[0] = str(widest + 1)
    return "/".join(f)


def tw_opts_raw(h):
    return "/".join([str(rng.choice([1, 2, 3, 5, 8, 10, 12, 15, 20, 30, 70, 0, -1])), h(rng.choice(["", "", "> ", "* ", "\t", "长"])),
                     h(rng.choice(["", "", "  ", "| ", "\t"])), rng.choice("110"), str(rng.choice([8, 8, 4, 0, 1, 3])), rng.choice("110"),
                     rng.choice("01"), rng.choice("110"), rng.choice("110"), rng.choice("110"), rng.choice(["~", "~", "~", "1", "2", "3", "0"]),
                     h(rng.choice([" [...]", " [...]", "...", "", " ", " ~~~~~~~~~~~~~", "　…"]))])


def tw_lines_text():
    lines = []
    for _ in range(rng.randrange(0, 6)):
        lines.append(rng.choice(["", " ", "\t", "    ", "  \t", "\t  "]) + rng.choice(["", "a b", "x", "  \t", "é", "#"]) + rng.choice(["", " ", "\r"]))
    return rng.choice(["\n", "\n", "\r\n", "\r", "\x0b", " ", "\x1c"]).join(lines) + rng.choice(["", "\n", "\n\n"])


def pytextwrap_line():
    r = rng.random()
    if r < 0.6:
        return f"wr {tw_text().encode().hex() or '-'} {tw_opts()}"
    if r < 0.75:
        return f"sh {tw_text().encode().hex() or '-'} {tw_opts()}"
    if r < 0.9:
        return f"de {tw_lines_text().encode().hex() or '-'}"
    return f"in {tw_lines_text().encode().hex() or '-'} {rng.choice(['> ', '', '\t', '#']).encode().hex() or '-'}"


family("pytextwrap", "pytextwrap", [pytextwrap_line() for _ in range(int(os.environ.get("PYTEXTWRAP_LINES", "6000")))],
       ["python3.13", os.path.join(LIB, "pytextwrap/oracle.py")])


# --- Python's calendar, against CPython 3.13 ------------------------------------------------------------------

def cal_year():
    return rng.choice([2024, 2000, 1900, 2100, 1, 9999, 0, -1, -400, 10000, 1752, 1970, rng.randrange(-3000, 12000)])


def pycalendar_line():
    r = rng.random()
    y = cal_year()
    m = rng.choice([1, 2, 6, 12, rng.randrange(1, 13), rng.randrange(1, 13), 0, 13])
    fw = rng.choice([0, 0, 6, 1, 3, 7, -1, 13])
    if r < 0.3:
        return f"fm {y} {m} {rng.choice([0, 2, 3, 4, 9, 10, 1, -1])} {rng.choice([0, 1, 1, 2])} {fw}"
    if r < 0.4:
        return f"fy {y} {rng.choice([0, 2, 3, 9])} {rng.choice([0, 1, 2])} {rng.choice([6, 2, 0, 1, 10])} {rng.choice([3, 3, 4, 2, 1, 5, 12])} {fw}"
    if r < 0.5:
        return f"hm {y} {m} {rng.choice([0, 1])} {fw}"
    if r < 0.55:
        return f"hy {y} {rng.choice([3, 4, 1, 0, -2, 12, 5])} {fw}"
    if r < 0.65:
        return f"mr {y} {m}"
    if r < 0.75:
        return f"mc {y} {m} {fw}"
    if r < 0.85:
        return f"wd {y} {m} {rng.choice([1, 15, 28, 29, 30, 31, 0, 32])}"
    if r < 0.9:
        return f"ld {cal_year()} {cal_year()}"
    if r < 0.93:
        return f"il {y}"
    return f"tg {y} {m} {rng.choice([1, 31, 0, -5, 400])} {rng.choice([0, 23, 24, -1])} {rng.choice([0, 59, 60])} {rng.choice([0, 59, 61, -1])}"


family("pycalendar", "pycalendar", [pycalendar_line() for _ in range(int(os.environ.get("PYCALENDAR_LINES", "4000")))],
       ["python3.13", os.path.join(LIB, "pycalendar/oracle.py")])


def any_double():
    r = rng.random()
    if r < 0.3:
        return rng.choice([0.0, -0.0, 0.1, 0.5, 1.5, 2.5, -2.5, 1.005, 1e21, 1e-7, 123.456, 5e-324, 1.7976931348623157e308, float("inf"), float("nan"),
                           1 / 3, 2 ** 53, 0.045])
    if r < 0.55:
        return struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]
    return rng.uniform(-1, 1) * 10 ** rng.randrange(-25, 25)


# --- Python's fractions, against CPython 3.13 -----------------------------------------------------------------

def fr_int(big=False):
    r = rng.random()
    if r < 0.4:
        return rng.choice([0, 1, -1, 2, 3, 7, 10, 12, 100, -3, 355, 113, 1000000])
    return rng.randrange(-10 ** (30 if big else 9), 10 ** (30 if big else 9))


FR_SPECIAL = ["1/8", "-1/8", "3/8", "5/2", "7/2", "1/2", "-5/2", "1/40", "3/16", "5/4", "1/1024", "125/1", "25/2", "-1/2000", "1/200000000",
              f"1/{10 ** 320}", f"3/{2 ** 1075}", f"-7/{2 ** 1080}", f"{10 ** 309}/1", f"{2 ** 1024 - 2 ** 970}/1", f"{2 ** 1024 - 2 ** 971}/1",
              f"1/{2 ** 1074}", f"1/{2 ** 1075}", f"3/{2 ** 1076}", f"{2 ** 53 + 1}/1", f"{10 ** 22 + 1}/{10 ** 22}"]


def fr_spec():
    if rng.random() < 0.15:
        return rng.choice(FR_SPECIAL)
    n = fr_int(rng.random() < 0.2)
    d = fr_int(rng.random() < 0.2) or 1
    from fractions import Fraction
    f = Fraction(n, d)
    return f"{f.numerator}/{f.denominator}"


FR_STRINGS = ["1", "-3/4", " 3/4 ", "3 / 4", "1.5", "-.5", "1.", ".5e3", "1e-3", "1E+2", "1_000", "1_000.000_1", "1__0", "_1", "1_", "3/0", "0/5", "abc",
              "", " ", "+", "1/2/3", "1.5/2", "1e", "1e+", "١٢", "١/٣", "　 5 　", "\t7\n", "0x10", "inf", "nan", "1/-2", "2e1_0", "+.5", "-0",
              "00012", "1.2.3", "1 .5", "1. 5", "²"]


def fr_format_spec():
    r = rng.random()
    if r < 0.15:
        return rng.choice(["", "10", "<10", "_^12", ",", "_", "+", " ", "#", "x>8", "=+12", "0", "05", "010", "z", "\n<5", "<<5", "^^", "->6"])
    fill = rng.choice(["", "", "", "*", "0", "x", "é", "<"])
    align = rng.choice(["", "", "", "<", ">", "^", "="])
    if fill and not align:
        fill = ""
    spec = fill + align + rng.choice(["", "", "+", "-", " "]) + rng.choice(["", "", "z"]) + rng.choice(["", "", "#"])
    spec += rng.choice(["", "", "0"]) + rng.choice(["", "", "8", "12", "1", "20"]) + rng.choice(["", "", ",", "_"])
    spec += rng.choice(["", "", ".0", ".2", ".3", ".10", ".1"]) + rng.choice(["e", "E", "f", "F", "g", "G", "%", "f", "g", "", "x", "n"])
    return spec


def pyfractions_line():
    r = rng.random()
    if r < 0.15:
        return f"fs {rng.choice(FR_STRINGS).encode().hex() or '-'}"
    if r < 0.22:
        return f"ff {dbits(any_double()):x}"
    if r < 0.4:
        k = rng.choice(["add", "sub", "mul", "div", "fdiv", "mod", "pow"])
        b = str(rng.randrange(-6, 7)) if k == "pow" else (fr_spec() if rng.random() < 0.9 else "0/1")
        return f"ar {k} {fr_spec()} {b}"
    if r < 0.48:
        return f"ld {fr_spec()} {rng.choice([1, 10, 100, 1000000, 0, -1, rng.randrange(1, 10 ** 6)])}"
    if r < 0.56:
        return f"rd {fr_spec()} {rng.choice(['~', '~', '0', '1', '2', '-1', '-2', '5'])}"
    if r < 0.64:
        return f"fl {fr_spec()}"
    if r < 0.95:
        return f"fm {fr_spec()} {fr_format_spec().encode().hex() or '-'}"
    return f"st {fr_spec()}"


family("pyfractions", "pyfractions", [pyfractions_line() for _ in range(int(os.environ.get("PYFRACTIONS_LINES", "6000")))],
       ["python3.13", os.path.join(LIB, "pyfractions/oracle.py")])


# --- run ------------------------------------------------------------------------------------------------

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
                print(f"  {fam} {ins[k][:200]}\n    want {want_line[:300]}\n    got  {have[:300]}")
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
