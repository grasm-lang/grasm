"""B56's packages against the references they reproduce.

- SBCL 2.6's ~R family: English cardinals and ordinals, Roman and old
  Roman numerals, and radix printing with padding, sign and commas.
- Rust 1.99's std::path::Path on Unix: components, parent, file name,
  stem, prefix and extension, join, with_file_name, with_extension,
  strip_prefix, starts_with and ends_with.
- Text::ParseWords 3.30 (Perl 5.34): parse_line, quotewords,
  nested_quotewords, shellwords and old_shellwords.
- Ruby 2.6's Float#to_s, round (half: up/even/down), floor, ceil and
  truncate, with digits.
- Rust 1.99's char/str escape_debug, escape_default, escape_unicode and
  `{:?}` of str and char.
- Text::Wrap 2013.0523 and Text::Tabs (Perl 5.34): wrap, fill, expand
  and unexpand, with columns, huge, unexpand, tabstop, separator,
  separator2 and break.
- CPython 3.13.1's ipaddress: addresses, networks and interfaces of both
  versions, their properties, arithmetic, ordering, subnets, supernet,
  address_exclude, summarize_address_range, collapse_addresses and
  format, with exception classes and messages.
- CPython 3.13's str methods: split, rsplit, splitlines, strip, lstrip,
  rstrip, expandtabs, partition, rpartition, center, ljust, rjust, zfill,
  find, rfind, index, rindex, count, startswith, endswith, replace,
  upper, lower, title, capitalize and swapcase, with every code point's
  case mappings checked against Python.
"""
import math
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B56_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B56_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B56_LIB") or os.path.join(ROOT, "scripts/lib/b56")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b56")
harness = os.path.join(work, "b56test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b56test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


# --- Common Lisp's ~R family, against SBCL 2.6 -------------------------------------------------------------------

def ln_number():
    r = rng.random()
    if r < 0.3:
        return rng.randrange(-1000, 1000)
    if r < 0.5:
        return rng.choice([0, 1, 12, 13, 19, 20, 21, 99, 100, 101, 110, 112, 1000, 1001, 1012, 3999, 4000, 4999, 5000, 10 ** 6, 10 ** 66 - 1, 10 ** 66,
                           10 ** 66 + 99, -(10 ** 66), 10 ** 66 + 112, 1100, 1200, -12, -100, 2 ** 63, 2 ** 64])
    if r < 0.8:
        return rng.randrange(-10 ** rng.randrange(1, 70), 10 ** rng.randrange(1, 70))
    return rng.randrange(1, 5100)


def lispnumbers_line():
    n = ln_number()
    r = rng.random()
    if r < 0.3:
        return f"lispnumbers c {n}"
    if r < 0.55:
        return f"lispnumbers o {n}"
    if r < 0.7:
        return f"lispnumbers r {n}"
    if r < 0.8:
        return f"lispnumbers or {n}"
    pad = rng.choice(["20", "30", "2a", "2e"])
    comma = rng.choice(["2c", "2e", "5f", "20"])
    return f"lispnumbers x {n} {rng.choice([10, 2, 8, 16, 36, rng.randrange(2, 37)])} {rng.choice([0, 1, 5, 12, 30])} {pad} {comma} {rng.choice([3, 1, 2, 4, 7])} " \
           f"{rng.choice([0, 1])} {rng.choice([0, 1])}"


family("lispnumbers", "lispnumbers", [lispnumbers_line() for _ in range(int(os.environ.get("LISPNUMBERS_LINES", "10000")))],
       ["sbcl", "--script", os.path.join(LIB, "lispnumbers/oracle.lisp")])


# --- Rust's std::path::Path on Unix, against Rust 1.99 ------------------------------------------------------------

RP_PARTS = ["a", "b", "c.txt", ".", "..", "", ".git", "x.tar.gz", "a.", "..a", "...", "\u00e9.rs", "name.with.dots", ".hidden.txt", "a b"]


def rp_path():
    head = rng.choice(["", "", "/", "./", "//", "../", "/./"])
    parts = [rng.choice(RP_PARTS) for _ in range(rng.randrange(0, 5))]
    tail = rng.choice(["", "", "/", "//", "/.", "/./"])
    return head + "/".join(parts) + tail


RUSTPATH = rust_tool("rustpath", os.path.join(LIB, "rustpath/oracle.rs"))


def rustpath_line():
    op = rng.choice(["comp", "parent", "name", "stem", "prefix", "ext", "abs", "join", "wfn", "wext", "strip", "starts", "ends", "comp", "parent", "strip"])
    a = rp_path()
    h = lambda t: t.encode().hex() or "-"
    if op in ("join", "wfn"):
        return f"rustpath {op} {h(a)} {h(rp_path() if rng.random() < 0.6 else rng.choice(RP_PARTS))}"
    if op == "wext":
        return f"rustpath {op} {h(a)} {h(rng.choice(['', 'md', 'tar.gz', '.x', 'a/b', '\u00e9', '..']))}"
    if op in ("strip", "starts", "ends"):
        b = a[:rng.randrange(0, len(a) + 1)] if rng.random() < 0.5 else (a[rng.randrange(0, len(a) + 1):] if op == "ends" else rp_path())
        return f"rustpath {op} {h(a)} {h(b)}"
    return f"rustpath {op} {h(a)}"


family("rustpath", "rustpath", [rustpath_line() for _ in range(int(os.environ.get("RUSTPATH_LINES", "10000")))], [RUSTPATH])


# --- Perl's Text::ParseWords, against Perl 5.34 -----------------------------------------------------------------

PW_ATOMS = ["a", "bc", " ", "  ", "\t", "\n", ",", ", ", ":", "|", "\"", "'", "\"x y\"", "'p q'", "\"a\\\"b\"", "'it\\'s'", "\\", "\\ ", "\\\"", "\\n", "\u00e9",
            "\u00a0", "\u2028", "\u3000", "\"\"", "''", "x\"y z\"w", "\"un", ";", "\\\\", "#", "\u000b", "\u000c"]
PW_DELIMS = ["\\s+", ",", "\\s*,\\s*", ":", "\\|", "[;,]", " ", "\\s", ",+", "\\t", "a", "x*", ";|,"]


def pw_line():
    return "".join(rng.choice(PW_ATOMS) for _ in range(rng.randrange(0, 7)))


def perlparsewords_line():
    h = lambda t: t.encode().hex() or "-"
    r = rng.random()
    keep = rng.choice(["0", "0", "1", "d"])
    if r < 0.4:
        return f"perlparsewords pl {h(rng.choice(PW_DELIMS))} {keep} {h(pw_line())}"
    if r < 0.55:
        return f"perlparsewords qw {h(rng.choice(PW_DELIMS))} {keep} " + " ".join(h(pw_line()) for _ in range(rng.randrange(1, 4)))
    if r < 0.65:
        return f"perlparsewords nq {h(rng.choice(PW_DELIMS))} {keep} " + " ".join(h(pw_line()) for _ in range(rng.randrange(1, 4)))
    if r < 0.85:
        return "perlparsewords sw " + " ".join(h(pw_line()) for _ in range(rng.randrange(1, 4)))
    return "perlparsewords os " + " ".join(h(pw_line()) for _ in range(rng.randrange(1, 3)))


family("perlparsewords", "perlparsewords", [perlparsewords_line() for _ in range(int(os.environ.get("PERLPARSEWORDS_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perlparsewords/oracle.pl")])


# --- Ruby's Float#to_s and rounding, against Ruby 2.6 -----------------------------------------------------------

def rf_value():
    r = rng.random()
    if r < 0.12:
        return struct.unpack(">d", rng.getrandbits(64).to_bytes(8, "big"))[0]
    if r < 0.17:
        return rng.choice([1, -1]) * rng.getrandbits(rng.randrange(1, 53)) * 5e-324
    if r < 0.4:
        d = rng.randrange(0, 8)
        return (rng.randrange(-10 ** 9, 10 ** 9) + rng.choice([0, 0.5, 0.25, 0.05, 0.005, 0])) / 10 ** d
    if r < 0.5:
        return rng.choice([0.0, -0.0, math.inf, -math.inf, math.nan, 5e-324, -5e-324, 2.2250738585072014e-308, 1.7976931348623157e308,
                           2.0 ** 53, 2.0 ** 53 + 2, 2.0 ** 62, -2.0 ** 62, 2.0 ** 63, 1e15, 1e16, 1e17, 1e-4, 1e-5, 9.999999999999999e15,
                           0.1, 0.5, 1.5, 2.5, -2.5, 1.005, 2.675, 1.0e20, 1.0e22, 123456789012345.6, 0.0001])
    if r < 0.7:
        return rng.choice([1, -1]) * rng.randrange(1, 10 ** 6) * 10.0 ** rng.randrange(-30, 30)
    if r < 0.85:
        return rng.choice([1, -1]) * (rng.randrange(0, 10 ** 4) + 0.5) * 10.0 ** rng.randrange(-6, 25)
    return rng.choice([1, -1]) * rng.random() * 10.0 ** rng.randrange(-320, 309)


def rf_cdiv(a, b):
    return int(a / b)


def rf_edge(v):
    """A digit count at one of Ruby's cut-offs for v, or None."""
    if v == 0 or not math.isfinite(v):
        return None
    e = math.frexp(v)[1]
    over = 17 - (rf_cdiv(e, 4) if e > 0 else rf_cdiv(e, 3) - 1)
    under = -(rf_cdiv(e, 3) + 1 if e > 0 else rf_cdiv(e, 4))
    return rng.choice([over, under]) + rng.choice([-1, 0, 1])


def rubyfloat_line():
    r = rng.random()
    if r < 0.08:
        # a decimal tie at the digit being rounded to
        n = rng.randrange(1, 12)
        frac = str(rng.randrange(0, 10 ** (n - 1))).zfill(n - 1) if n > 1 else ""
        v = float(f"{rng.choice(['', '-'])}{rng.randrange(0, 10 ** rng.randrange(1, 7))}.{frac}5")
        return f"rubyfloat rd {struct.pack('>d', v).hex()} {n - 1 if n > 1 and rng.random() < 0.2 else n} {rng.choice(['up', 'even', 'down'])}"
    if r < 0.14:
        # an integer tie for negative digits
        m = rng.randrange(1, 22)
        v = float(rng.choice([1, -1]) * (rng.randrange(0, 10 ** 4) * 10 ** m + 5 * 10 ** (m - 1)))
        return f"rubyfloat rd {struct.pack('>d', v).hex()} {-m} {rng.choice(['up', 'even', 'down'])}"
    v = rf_value()
    h = struct.pack(">d", v).hex()
    r = rng.random()
    n = rng.choice([rng.randrange(-25, 26), rng.randrange(-25, 26), rng.randrange(-3, 18), rng.randrange(-400, 401), rng.randrange(240, 360)])
    if rng.random() < 0.3:
        n = rf_edge(v) or n
    if r < 0.25:
        return f"rubyfloat ts {h}"
    if r < 0.55:
        return f"rubyfloat rd {h} {n} {rng.choice(['up', 'even', 'down'])}"
    if r < 0.6:
        return f"rubyfloat r0 {h}"
    return f"rubyfloat {rng.choice(['fl', 'ce', 'tr'])} {h} {n}"


family("rubyfloat", "rubyfloat", [rubyfloat_line() for _ in range(int(os.environ.get("RUBYFLOAT_LINES", "10000")))],
       ["ruby", os.path.join(LIB, "rubyfloat/oracle.rb")])


# --- Rust's char/str escapes and {:?}, against Rust 1.99 ---------------------------------------------------------

RE_SETS = {}
for _l in open(os.path.join(LIB, "rustescape/sets.txt")):
    _f = _l.split()
    RE_SETS[_f[0]] = [(int(_f[i]), int(_f[i + 1])) for i in range(1, len(_f), 2)]


def re_char():
    r = rng.random()
    if r < 0.25:
        return chr(rng.randrange(0, 128))
    if r < 0.32:
        return rng.choice("\0\t\r\n\\'\"\x7f\x1b ")
    if r < 0.6:
        # near an edge of one of the two sets
        lo, hi = rng.choice(RE_SETS[rng.choice("NG")])
        cp = rng.choice([lo - 1, lo, hi, hi + 1, rng.randrange(lo, hi + 1)])
    elif r < 0.8:
        cp = rng.randrange(0x80, 0x3000)
    else:
        cp = rng.randrange(0, 0x110000)
    if 0xD800 <= cp <= 0xDFFF or cp < 0 or cp > 0x10FFFF:
        cp = 0x301
    return chr(cp)


def rustescape_line():
    s = "".join(re_char() for _ in range(rng.randrange(0, 6)))
    op = rng.choice(["cd", "cf", "cu", "sd", "sf", "su", "ds", "dc", "sd", "ds", "cd", "dc"])
    if op[0] == "c" or op == "dc":
        s = s[:1] or re_char()
    return f"rustescape {op} {s.encode().hex() or '-'}"


RUSTESCAPE = rust_tool("rustescape", os.path.join(LIB, "rustescape/oracle.rs"))
def rustescape_edges():
    """Every edge of both sets, each side, through char escape_debug."""
    out = []
    for lo, hi in RE_SETS["N"] + RE_SETS["G"]:
        for cp in (lo - 1, lo, hi, hi + 1):
            if 0 <= cp <= 0x10FFFF and not 0xD800 <= cp <= 0xDFFF:
                out.append(f"rustescape {rng.choice(['cd', 'dc'])} {chr(cp).encode().hex()}")
    return out


family("rustescape", "rustescape", rustescape_edges() + [rustescape_line() for _ in range(int(os.environ.get("RUSTESCAPE_LINES", "10000")))], [RUSTESCAPE])


# --- Perl's Text::Wrap and Text::Tabs, against Perl 5.34 -----------------------------------------------------------

# Characters and sequences that Unicode 13 (Perl 5.34) and 17 (packages/graphemes) cluster alike.
TW_ATOMS = ["a", "bb", "word", "longerword", "x" * 12, "supercalifragilistic", " ", " ", "  ", "   ", "\t", "\n", "\n\n", "\r\n", "e\u0301", "\u0301",
            "\u4e2d\u6587", "\u00a0", "\u3000", "\u2028", "\u0085", "\u000b", "\u000c", "-", ":", ",", "\U0001f44d\U0001f3fd", "\U0001f469\u200d\U0001f4bb",
            "\U0001f1e6\U0001f1ea", "\u1100\u1161\u11a8", "\u00e9", " \u0301", "\t\t", "ab\tc", "\u00ad", "\u200b", "-\n", ":\n", ";\n", ",\n"]
TW_BREAKS = ["u", "u", "u", r"\s", r"[\s:]", r"[\s,]", r"\s+", "-", r"[\s-]", r"\s|-", "[:;]"]


def tw_text(n):
    return "".join(rng.choice(TW_ATOMS) for _ in range(rng.randrange(0, n)))


def tw_hex(t):
    return t.encode().hex() or "-"


def perltextwrap_line():
    r = rng.random()
    if r < 0.12:
        return f"perltextwrap {rng.choice(['e', 'x'])} {rng.choice([8, 8, 4, 1, 2, 3, 5])} {tw_hex(tw_text(10))}"
    columns = rng.choice([2, 3, 5, 8, 10, 12, 16, 20, 30, 76, rng.randrange(2, 40)])
    huge = rng.choice(["wrap", "wrap", "overflow", "die"])
    sep = rng.choice(["\n", "\n", "\n", "|", "\r\n", "<br>"])
    sep2 = rng.choice([None, None, None, "\n", "~"])
    brk = rng.choice(TW_BREAKS)
    ip = rng.choice(["", "", "  ", "\t", "* ", "> ", "\u00b7 ", "        ", "abcdefghijkl"])
    xp = rng.choice(["", "", "  ", "\t", "  * ", "    ", "\t\t", ">"])
    texts = [tw_text(rng.choice([4, 8, 16])) for _ in range(rng.randrange(1, 4))]
    op = "w" if rng.random() < 0.75 else "f"
    return " ".join(["perltextwrap", op, str(columns), huge, rng.choice(["1", "1", "0"]), str(rng.choice([8, 8, 4, 2, 3])), tw_hex(sep),
                     "u" if sep2 is None else tw_hex(sep2), "u" if brk == "u" else tw_hex(brk), tw_hex(ip), tw_hex(xp)] + [tw_hex(t) for t in texts])


family("perltextwrap", "perltextwrap", [perltextwrap_line() for _ in range(int(os.environ.get("PERLTEXTWRAP_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perltextwrap/oracle.pl")])


# --- Python's ipaddress, against CPython 3.13.1 -----------------------------------------------------------------------

PI_V4_EDGES = ["0.0.0.0", "255.255.255.255", "10.0.0.0", "9.255.255.255", "11.0.0.0", "100.64.0.0", "100.63.255.255", "100.127.255.255", "100.128.0.0",
               "127.0.0.1", "126.255.255.255", "128.0.0.0", "169.254.0.0", "169.254.255.255", "169.253.255.255", "172.16.0.0", "172.31.255.255",
               "172.32.0.0", "172.15.255.255", "192.0.0.0", "192.0.0.8", "192.0.0.9", "192.0.0.10", "192.0.0.11", "192.0.0.170", "192.0.0.171",
               "192.0.0.172", "192.0.0.255", "192.0.1.0", "192.0.2.0", "192.0.2.255", "192.168.0.0", "192.168.255.255", "198.18.0.0", "198.19.255.255",
               "198.20.0.0", "198.51.100.1", "203.0.113.255", "224.0.0.0", "223.255.255.255", "239.255.255.255", "240.0.0.0", "255.255.255.254",
               "8.8.8.8", "1.2.3.4", "0.0.0.1", "0.255.255.255", "1.0.0.0"]
PI_V6_EDGES = ["::", "::1", "::2", "::ffff:0:0", "::ffff:1.2.3.4", "::ffff:10.0.0.1", "::ffff:0.0.0.0", "::ffff:127.0.0.1", "::ffff:224.0.0.1",
               "::ffff:240.0.0.1", "::ffff:169.254.1.1", "::ffff:100.64.0.1", "::fffe:0:0", "::1:ffff:0:0", "64:ff9b:1::", "64:ff9b:1:ffff::",
               "64:ff9b::1", "100::", "100::ffff:ffff:ffff:ffff", "100:0:0:1::", "2001::", "2001:1ff:ffff::", "2001:200::", "2001:1::1", "2001:1::2",
               "2001:1::3", "2001:2::", "2001:3::", "2001:3:ffff::", "2001:4:112::", "2001:4:113::", "2001:20::", "2001:2f:ffff::", "2001:30::",
               "2001:3f::", "2001:40::", "2001:db8::", "2001:db8:ffff:ffff::", "2001:db9::", "2001:0:4136:e378:8000:63bf:3fff:fdd2", "2002::",
               "2002:c000:204::1", "2002:ffff::", "2003::", "3fff::", "3fff:fff::", "3fff:1000::", "fc00::", "fdff:ffff::", "fe00::", "fe7f::",
               "fe80::", "febf:ffff::", "fec0::", "feff::", "ff00::", "ff02::1", "fe80::1", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", "1::", "200::",
               "400::", "800::", "1000::", "2000::", "4000::", "6000::", "8000::", "a000::", "c000::", "e000::", "f000::", "f800::", "fe00::1"]
PI_SCOPES = ["eth0", "1", "lo", "a.b", "x:y", "é", "中", "a b", "\t", "0", "%", "", "a%b", "/", "\n", "\x00", "​", "\u0897", "\u1c89", "\U0001fbef", "\u0378"]


def pi_v4_text():
    r = rng.random()
    if r < 0.3:
        return rng.choice(PI_V4_EDGES)
    if r < 0.8:
        return ".".join(str(rng.choice([0, 255, rng.randrange(256), rng.randrange(256), 1, 10, 100])) for _ in range(4))
    parts = [str(rng.randrange(256)) for _ in range(4)]
    k = rng.randrange(4)
    bad = rng.choice(["", "01", "00", "000", "256", "999", "1000", "0255", "a", "+1", "-1", " 1", "1 ", "１", "١", "²", "0x1", "1e1", "1_0",
                      "%1", "1/2", "é", "\n", "\u1c89", "\u0378"])
    parts[k] = bad
    if rng.random() < 0.25:
        n = rng.choice([1, 2, 3, 5, 6])
        parts = (parts * 2)[:n]
    return ".".join(parts)


def pi_hextet():
    r = rng.random()
    if r < 0.35:
        return "0"
    if r < 0.5:
        return rng.choice(["ffff", "1", "a", "FFFF", "0000", "00", "abcd", "ABCD", "fe80", "2001", "db8"])
    return "%x" % rng.randrange(0x10000) if rng.random() < 0.8 else "%04X" % rng.randrange(0x10000)


def pi_v6_core():
    r = rng.random()
    if r < 0.25:
        return rng.choice(PI_V6_EDGES)
    groups = [pi_hextet() for _ in range(8)]
    if rng.random() < 0.15:
        groups[5] = "ffff"
        for i in range(5):
            groups[i] = "0"
    if r < 0.45:
        return ":".join(groups)
    if r < 0.85:
        # compress some run
        a = rng.randrange(0, 9)
        b = rng.randrange(a, 9)
        if b - a == 8:
            return "::"
        left = ":".join(groups[:a])
        right = ":".join(groups[b:]) if b < 8 else ""
        if a == b:
            b = min(8, a + 1)
            right = ":".join(groups[b:])
        return left + "::" + right
    # an IPv4 tail
    head = groups[:6]
    tail = pi_v4_text() if rng.random() < 0.4 else ".".join(str(rng.randrange(256)) for _ in range(4))
    if rng.random() < 0.5:
        a = rng.randrange(0, 7)
        return ":".join(head[:a]) + "::" + ":".join(head[min(6, a + rng.randrange(1, 3)):]) + (":" if a < 6 and rng.random() < 0.7 else "") + tail
    return ":".join(head) + ":" + tail


def pi_v6_bad(t):
    r = rng.random()
    if r < 0.15:
        return t + rng.choice([":", "::", ":1", ":g"])
    if r < 0.3:
        return rng.choice([":", "", "::"]) + t
    if r < 0.45:
        i = rng.randrange(len(t) + 1)
        return t[:i] + rng.choice(["g", "::", ":", "12345", "0000", "１", " ", ".", "-", "_"]) + t[i:]
    if r < 0.6:
        return rng.choice([":::", "1::2::3", "1:2:3:4:5:6:7:8:9", ":1:2:3:4:5:6:7:8", "1:2:3:4:5:6:7:", "1:2:3:4:5:6:7::8", "::1:2:3:4:5:6:7:8",
                           "1:2", "1", "1:2:3", "::1.2.3", "::1.2.3.04", "::1.2.3.4.5", "1.2.3.4::", "::ffff:1.2.3.4:5", "1:2:3:4:5:6:7:1.2.3.4",
                           "::1.2.3.256", "fe80:::1", "12345::", "::12345", "1::2:3:4:5:6:7:8", "1:2:3:4::5:6:7:8", ":1::", "::1:", "1:2::3:"])
    if r < 0.75:
        return t.replace(":", ".", 1)
    return t.upper() if rng.random() < 0.5 else t + ":" + pi_hextet()


def pi_v6_text():
    t = pi_v6_core()
    if rng.random() < 0.15:
        t = pi_v6_bad(t)
    if rng.random() < 0.12:
        t += "%" + rng.choice(PI_SCOPES)
    return t


def pi_addr_text(v=None):
    v = v or rng.choice([4, 6])
    return pi_v4_text() if v == 4 else pi_v6_text()


V4_MASKS = ["255.255.255.0", "255.255.0.0", "255.0.0.0", "0.0.0.0", "255.255.255.255", "255.255.255.128", "255.255.254.0", "0.0.0.255", "0.0.255.255",
            "0.255.255.255", "0.0.0.1", "0.0.0.3", "255.255.255.254", "255.0.255.0", "0.255.0.255", "128.0.0.0", "127.255.255.255", "255.255.0.255",
            "256.0.0.0", "255.255.255", "01.0.0.0"]


def pi_prefix(v):
    m = 32 if v == 4 else 128
    r = rng.random()
    if r < 0.55:
        return str(rng.randrange(0, m + 1))
    if r < 0.7:
        return str(rng.choice([0, 1, m - 1, m, m - 8, 8, 16, 24, 64, 96, 120, 127]))
    if r < 0.85 and v == 4:
        return rng.choice(V4_MASKS)
    return rng.choice(["", "33", "129", "-1", "+8", "024", "0000000000024", "8 ", " 8", "a", "８", "1e1", "99999999999999999999", "0x8", "0", "00",
                       "128", "32"])


PI_NET_EDGES = {4: ["0.0.0.0/0", "0.0.0.0/32", "0.0.0.0", "0.0.0.0/8", "127.0.0.0/8", "127.0.0.1/32", "127.0.0.1", "100.64.0.0/10", "100.64.0.0/11",
                    "100.0.0.0/8", "192.0.0.8/29", "192.0.0.9/32", "192.0.0.10/31", "192.0.0.170/31", "192.0.0.0/24", "192.0.0.0/23", "224.0.0.0/4",
                    "224.0.0.0/3", "240.0.0.0/4", "240.0.0.0/3", "255.255.255.255/32", "169.254.0.0/16", "169.254.0.0/15", "10.0.0.0/8", "10.0.0.0/7",
                    "1.2.3.4/31", "1.2.3.5/31", "1.2.3.4/30", "1.2.3.4/32"],
                6: ["::/0", "::/128", "::", "::1/128", "::1", "::/127", "::ffff:0:0/96", "::ffff:0:0/95", "::ffff:0.0.0.0/128", "::ffff:127.0.0.1/128",
                    "::ffff:127.0.0.0/104", "::ffff:224.0.0.0/100", "2001::/23", "2001::/22", "2001:1::1/127", "2001:1::/126", "2001:3::/32",
                    "2001:4:112::/48", "2001:20::/28", "2001:30::/27", "fe80::/10", "fe80::/9", "fec0::/10", "fec0::/9", "ff00::/8", "fe00::/7",
                    "fc00::/7", "fc00::/6", "2002::/16", "64:ff9b:1::/48", "::/8", "::/7", "100::/64", "100::/63", "3fff::/20", "2001:db8::/32",
                    "1::/127", "1::1/127", "1::/128"]}


def pi_net_text(v=None, aligned=None):
    v = v or rng.choice([4, 6])
    if rng.random() < 0.15:
        return rng.choice(PI_NET_EDGES[v])
    a = pi_addr_text(v)
    if rng.random() < 0.1:
        return a
    p = pi_prefix(v)
    if aligned or (aligned is None and rng.random() < 0.5):
        # mask the address in Python so that strict networks are often valid
        try:
            n = __import__("ipaddress").ip_network(a.split("%")[0] + "/" + p, strict=False)
            a = str(n.network_address) + (("%" + a.split("%", 1)[1]) if "%" in a and v == 6 else "")
        except ValueError:
            pass
    t = a + "/" + p
    if rng.random() < 0.04:
        t += "/" + pi_prefix(v)
    return t


def pi_hx(t):
    return t.encode("utf-8").hex() or "-"


def pi_valid_net(v=None, lo=0, hi=None):
    """A network text that parses (non-strict), with a prefix in [lo, hi]."""
    import ipaddress
    v = v or rng.choice([4, 6])
    m = 32 if v == 4 else 128
    hi = m if hi is None else hi
    while True:
        a = pi_addr_text(v).split("%")[0]
        try:
            ipaddress.ip_address(a)
        except ValueError:
            continue
        p = rng.randrange(lo, hi + 1)
        return str(ipaddress.ip_network(f"{a}/{p}", strict=False))


def pi_ctor(v, network=False):
    """Mostly the constructor for the text's version, sometimes the other."""
    c = rng.choice(["ip", "ip", "ip", f"v{v}", f"v{v}", f"v{v}", f"v{v}", f"v{10 - v}"])
    return c + rng.choice(["s", "n"]) if network else c


def pi_arg(kind, text, ctor=None):
    if kind == "N":
        ctor = ctor or rng.choice(["ipn", "ipn", "ips", "v4n" if ":" not in text else "v6n"])
    else:
        ctor = ctor or rng.choice(["ip", "ip", "ip", "v4" if ":" not in text else "v6"])
    return f"{kind}:{ctor}:{pi_hx(text)}"


def pi_big():
    return rng.choice([0, 1, -1, 2, -2, 255, 256, 2 ** 32 - 1, 2 ** 32, 2 ** 31, 2 ** 64, 2 ** 96, 2 ** 128 - 1, 2 ** 128, -(2 ** 128), 2 ** 200,
                       rng.randrange(-2 ** 40, 2 ** 40), rng.randrange(0, 2 ** 130), rng.randrange(-100, 100)])


def pi_inside(net_text, scoped=False):
    """A subnet of the network text, or something near it."""
    import ipaddress
    n = ipaddress.ip_network(net_text)
    m = n.max_prefixlen
    p = rng.randrange(n.prefixlen, m + 1)
    off = rng.randrange(n.num_addresses)
    s = ipaddress.ip_network(f"{n.network_address + off}/{p}", strict=False)
    t = str(s)
    if scoped and n.version == 6:
        t = str(s.network_address) + "%" + rng.choice(["x", "eth0"]) + f"/{s.prefixlen}"
    return t


def pyipaddress_line():
    import ipaddress
    r = rng.random()
    if r < 0.22:
        v = rng.choice([4, 6])
        return f"pyipaddress A {pi_ctor(v)} {pi_hx(pi_addr_text(v))}"
    if r < 0.40:
        v = rng.choice([4, 6])
        return f"pyipaddress N {pi_ctor(v, True)} {pi_hx(pi_net_text(v))}"
    if r < 0.52:
        v = rng.choice([4, 6])
        return f"pyipaddress I {pi_ctor(v)} {pi_hx(pi_net_text(v, aligned=False))}"
    if r < 0.56:
        v = rng.choice(["4", "6"])
        if rng.random() < 0.5:
            return f"pyipaddress AI {v} {rng.choice([pi_big(), rng.randrange(0, 2 ** (32 if v == '4' else 128))])}"
        n = rng.choice([4, 16, 4, 16, 0, 3, 5, 15, 17])
        return f"pyipaddress AP {v} {bytes(rng.randrange(256) for _ in range(n)).hex() or '-'}"
    if r < 0.61:
        if rng.random() < 0.3:
            v = rng.choice([4, 6])
            arg = pi_arg("I", pi_net_text(v, aligned=False), rng.choice(["ip", f"v{v}"]))
        else:
            arg = pi_arg("A", pi_addr_text())
        return f"pyipaddress {rng.choice(['ADD', 'SUB'])} {arg} {pi_big() if rng.random() < 0.6 else rng.randrange(-5, 6)}"
    if r < 0.69:
        kind = rng.choice(["A", "N", "I"])
        v = rng.choice([4, 6])
        if kind == "A":
            a = pi_addr_text(v)
            b = rng.choice([a, a.split("%")[0], a.split("%")[0] + "%x", pi_addr_text(v), pi_addr_text(), pi_succ(a)])
            return f"pyipaddress CMP {pi_arg('A', a, 'ip')} {pi_arg('A', b, 'ip')}"
        ctor = "ipn" if kind == "N" else "ip"
        a = pi_valid_net(v)
        base = a.split("/")[0]
        b = rng.choice([a, pi_valid_net(v), pi_valid_net(), base + "/" + str(rng.randrange(0, 33 if v == 4 else 129)), base + ("%x/" if v == 6 else "/") +
                        a.split("/")[1]])
        if kind == "I":
            a = pi_addr_text(v).split("%")[0] + "/" + a.split("/")[1] if rng.random() < 0.5 else a
        return f"pyipaddress CMP {pi_arg(kind, a, ctor)} {pi_arg(kind, b, ctor)}"
    if r < 0.73:
        n = pi_valid_net()
        nn = ipaddress.ip_network(n)
        i = rng.choice([0, -1, 1, -2, nn.num_addresses - 1, nn.num_addresses, -nn.num_addresses, -nn.num_addresses - 1, rng.randrange(-10, 10), pi_big()])
        return f"pyipaddress GI {pi_arg('N', n, 'ipn')} {i}"
    if r < 0.78:
        v = rng.choice([4, 6])
        m = 32 if v == 4 else 128
        n = pi_valid_net(v, lo=rng.choice([0, m - 4, rng.randrange(m + 1)]))
        p = int(n.split("/")[1])
        new = rng.choice(["-", "-", "-", str(rng.randrange(p - 2, m + 3)), str(m), str(p)])
        diff = rng.choice([1, 1, 2, 0, -1, 3, 8, m, m - p, m - p + 1]) if new == "-" else 1
        if new != "-" and rng.random() < 0.1:
            diff = rng.choice([0, 2])
        if rng.random() < 0.15:
            n = pi_net_text(v)
        return f"pyipaddress SUBN {pi_arg('N', n, 'ipn')} {diff} {new}"
    if r < 0.82:
        v = rng.choice([4, 6])
        m = 32 if v == 4 else 128
        n = pi_valid_net(v, lo=rng.choice([0, 0, 1, rng.randrange(m + 1)]))
        p = int(n.split("/")[1])
        new = rng.choice(["-", "-", "-", str(rng.randrange(-2, p + 3)), "0", str(p)])
        diff = rng.choice([1, 1, 2, 0, -1, p, p + 1, rng.randrange(-3, m + 2)]) if new == "-" or rng.random() < 0.1 else 1
        if rng.random() < 0.1:
            n = n.split("/")[0] + ("%x" if v == 6 else "") + "/" + str(p)
        return f"pyipaddress SUPN {pi_arg('N', n, 'ipn')} {diff} {new}"
    if r < 0.86:
        v = rng.choice([4, 6])
        m = 32 if v == 4 else 128
        a = pi_valid_net(v, lo=rng.choice([0, m - 6, rng.randrange(m + 1)]))
        rr = rng.random()
        if rr < 0.7:
            b = pi_inside(a, scoped=rng.random() < 0.1)
        elif rr < 0.8:
            b = a
        else:
            b = pi_valid_net(rng.choice([v, v, 10 - v]))
        if rng.random() < 0.05:
            a = a.split("/")[0] + ("%x" if v == 6 else "") + "/" + a.split("/")[1]
        return f"pyipaddress EXC {pi_arg('N', a, 'ipn')} {pi_arg('N', b, 'ipn')}"
    if r < 0.89:
        v = rng.choice([4, 6])
        a = pi_valid_net(v)
        b = rng.choice([pi_inside(a), pi_valid_net(v), pi_valid_net(), a, pi_inside(a, scoped=True)])
        if rng.random() < 0.5:
            a, b = b, a
        return f"pyipaddress NN {pi_arg('N', a, 'ipn')} {pi_arg('N', b, 'ipn')}"
    if r < 0.91:
        v = rng.choice([4, 6])
        a = pi_valid_net(v)
        x = rng.choice([pi_inside(a).split("/")[0], pi_addr_text(v), pi_addr_text()])
        return f"pyipaddress IN {pi_arg('N', a, 'ipn')} {pi_arg('A', x, 'ip')}"
    if r < 0.95:
        v = rng.choice([4, 6])
        m = 32 if v == 4 else 128
        lo = rng.randrange(2 ** m)
        if rng.random() < 0.3:
            lo = rng.choice([0, 2 ** m - 1, 2 ** m - 2, 1, lo & ~((1 << rng.randrange(m)) - 1)])
        span = rng.choice([0, 1, 2, rng.randrange(1000), rng.randrange(2 ** rng.randrange(1, m + 1)), 2 ** m])
        hi = min(lo + span, 2 ** m - 1)
        if rng.random() < 0.1:
            lo, hi = hi, lo
        A = ipaddress.IPv4Address if v == 4 else ipaddress.IPv6Address
        first, last = str(A(lo)), str(A(hi))
        if rng.random() < 0.08:
            last = pi_addr_text(10 - v).split("%")[0]
        if v == 6 and rng.random() < 0.05:
            first, last = first + "%a", first + "%b"
        return f"pyipaddress SAR {pi_arg('A', first, 'ip')} {pi_arg('A', last, 'ip')}"
    if r < 0.98:
        v = rng.choice([4, 6])
        m = 32 if v == 4 else 128
        base = int(ipaddress.ip_network(pi_valid_net(v, lo=m - 6, hi=m - 3)).network_address)
        A = ipaddress.IPv4Address if v == 4 else ipaddress.IPv6Address
        items = []
        mixed = rng.random() < 0.08
        for _ in range(rng.randrange(1, 9)):
            vv = 10 - v if mixed and rng.random() < 0.3 else v
            mm = 32 if vv == 4 else 128
            bb = base if vv == v else int(ipaddress.ip_network(pi_valid_net(vv, lo=mm - 6, hi=mm - 3)).network_address)
            AA = ipaddress.IPv4Address if vv == 4 else ipaddress.IPv6Address
            x = AA((bb + rng.randrange(64)) % 2 ** mm)
            if rng.random() < 0.5:
                items.append(pi_arg("A", str(x), "ip"))
            else:
                items.append(pi_arg("N", f"{x}/{rng.randrange(mm - 6, mm + 1)}", "ipn"))
        return "pyipaddress COL " + " ".join(items)
    kind = rng.choice(["A", "A", "I"])
    v = rng.choice([4, 6])
    t = pi_addr_text(v) if kind == "A" else pi_net_text(v, aligned=False)
    spec = rng.choice(["", "s", "b", "x", "X", "n", "#b", "#x", "#X", "#n", "_b", "_x", "_X", "_n", "#_b", "#_x", "#_X", "#_n", "20s", ">50s", "<45s",
                       "^60s", "*^50s", ".5s", "-<44s", "d", "o", "c", "10x", "#", "_", "__x", "##x", "_#x", "xb", "e", "N", "B"])
    return f"pyipaddress FMT {pi_arg(kind, t, rng.choice(['ip', f'v{v}']))} {pi_hx(spec)}"


def pi_succ(a):
    try:
        return str(__import__("ipaddress").ip_address(a.split("%")[0]) + 1)
    except ValueError:
        return a


def pi_ok(a):
    try:
        __import__("ipaddress").ip_address(a.split("%")[0])
        return True
    except ValueError:
        return False


def pi_col_mixed(line):
    """collapse_addresses with addresses of one version and networks of the
    other: Python's message depends on its sort's comparison order."""
    import ipaddress
    f = line.split(" ")
    if f[1] != "COL":
        return False
    av, nv = set(), set()
    for s in f[2:]:
        k, c, h = s.split(":")
        t = bytes.fromhex(h).decode()
        if k == "A":
            av.add(ipaddress.ip_address(t).version)
            continue
        n = ipaddress.ip_network(t, strict=False)
        (av if n.prefixlen == n.max_prefixlen else nv).add(n.version)
    return len(av) == 1 and len(nv) == 1 and av != nv


def pyipaddress_lines(count):
    out = []
    while len(out) < count:
        l = pyipaddress_line()
        if not pi_col_mixed(l):
            out.append(l)
    return out


family("pyipaddress", "pyipaddress", pyipaddress_lines(int(os.environ.get("PYIPADDRESS_LINES", "10000"))),
       ["python3.13", os.path.join(LIB, "pyipaddress/oracle.py")])


# --- Python's str methods, against CPython 3.13 -----------------------------------------------------------------

# The committed case table is what the generator writes today (it also writes the character pools used below).
PS_GEN = subprocess.run(["python3.13", os.path.join(LIB, "pystr/gen_tables.py")], capture_output=True, text=True).stdout
if not open(os.path.join(PKG, "pystr/src/main.gr"), encoding="utf-8").read().endswith(PS_GEN):
    print("pystr/src/main.gr: the generated case table is stale; run pystr/gen_tables.py")
    sys.exit(1)
PS_POOLS = __import__("json").load(open(os.path.join(LIB, "pystr/pools.json")))
PS_SPACES = "\t\n\x0b\x0c\r\x1c\x1d\x1e\x1f \x85\xa0                　"
PS_NOT_SPACES = "​᠎﻿⁠\x00\x7f_"
PS_BREAKS = ["\n", "\r", "\r\n", "\x0b", "\x0c", "\x1c", "\x1d", "\x1e", "\x85", " ", " ", "\n\r", "\x1f"]
PS_SPECIAL = ["ß", "ŉ", "ǰ", "ΐ", "ﬁ", "İ", "ǅ", "Ǆ", "ǆ", "ᾳ", "ᾼ", "Σ", "ς", "σ",
              "ẞ", "և", "ა", "Ა", "Ꭰ", "ꭰ", "\U00010400", "\U00010428", "\U0001e900", "ͅ", "ᾀ", "ᾈ",
              "ı", "ſ", "K", "Ω", "µ", "ẛ", "ﬅ", "Ι", "ι", "ι", "Ⅰ", "Ⓐ", "ⓐ"]
PS_WORDY = ["a", "b", "Z", "hello", "World", "ABC", "x1", "1", "42", "-", "+", "'", "’", ".", ":", "_", " ", "  ", "\t", "é", "É", "中",
            "\U0001f600", "́", "ö", "ab", "ba", "aba", "aa", "σοφια", "ΟΔΟΣ"]


def ps_pick(pool):
    return chr(rng.choice(PS_POOLS[pool]))


def ps_atom():
    r = rng.random()
    if r < 0.3:
        return rng.choice(PS_WORDY)
    if r < 0.42:
        return rng.choice(PS_SPACES)
    if r < 0.47:
        return rng.choice(PS_NOT_SPACES)
    if r < 0.55:
        return rng.choice(PS_BREAKS)
    if r < 0.65:
        return rng.choice(PS_SPECIAL)
    if r < 0.85:
        return ps_pick(rng.choice(["cased", "ignorable", "upper", "lower", "title", "special", "both", "cased", "ignorable"]))
    if r < 0.9:
        return "Σ"
    cp = rng.randrange(0x110000)
    return chr(cp) if not 0xD800 <= cp <= 0xDFFF else "Σ"


def ps_text(n=8):
    return "".join(ps_atom() for _ in range(rng.randrange(0, n)))


def ps_sigma_text():
    """A capital sigma with cased and case-ignorable characters around it."""
    side = lambda: "".join(rng.choice([ps_pick("cased"), ps_pick("ignorable"), ps_pick("ignorable"), ps_pick("both"), "'", ".", " ", "­", "A", "1",
                                        "Σ"]) for _ in range(rng.randrange(0, 3)))
    return side() + "Σ" + side() + (rng.choice(["", "Σ", " Σ"]) if rng.random() < 0.3 else "")


def ps_sub(s):
    r = rng.random()
    if r < 0.05:
        return ""
    if r < 0.6 and s:
        a = rng.randrange(len(s))
        return s[a:a + rng.randrange(1, 4)]
    if r < 0.75:
        return rng.choice(["a", "aa", " ", "\N{GREEK CAPITAL LETTER SIGMA}", "ab", "\n", "\r\n"])
    return ps_text(3) or "a"


def ps_bound():
    return rng.choice(["N", "N", "N", str(rng.randrange(-12, 13)), str(rng.choice([0, -1, 1, 100, -100]))])


PS_OPS = ["sp", "sp", "rsp", "rsp", "sl", "strip", "lstrip", "rstrip", "exp", "part", "rpart", "center", "ljust", "rjust", "zfill", "find", "rfind",
          "index", "rindex", "count", "startswith", "endswith", "replace", "upper", "lower", "lower", "title", "title", "capitalize", "swapcase"]


def ps_repetitive():
    """Repeats of a short pattern, where matches can overlap."""
    unit = rng.choice(["a", "ab", "aab", "\u03a3", "\u03a3\u03c3", "\r\n", " a"])
    return rng.choice(["", "x", "b"]) + unit * rng.randrange(1, 7) + rng.choice(["", "a", "y"])


def pystr_line():
    op = rng.choice(PS_OPS)
    s = ps_sigma_text() if op in ("lower", "title", "capitalize", "swapcase") and rng.random() < 0.4 else ps_text(rng.choice([4, 8, 14]))
    if op in ("sp", "rsp", "part", "rpart", "find", "rfind", "index", "rindex", "count", "replace") and rng.random() < 0.25:
        s = ps_repetitive()
    h = lambda t: t.encode().hex() or "-"
    if op in ("sp", "rsp"):
        sep = rng.choice(["N", "N", "N", h(ps_sub(s)), h(rng.choice([",", "a", "aa", " ", "Σ", ""]))])
        if sep == "-" and rng.random() < 0.7:
            sep = "N"
        return f"pystr {op} {h(s)} {sep} {rng.choice([-1, -1, 0, 1, 2, 3, -5, 100])}"
    if op == "sl":
        return f"pystr sl {h(s)} {rng.choice(['0', '1'])}"
    if op in ("strip", "lstrip", "rstrip"):
        if rng.random() < 0.5:
            chars = None
            pad = lambda: "".join(rng.choice(PS_SPACES + PS_NOT_SPACES) for _ in range(rng.randrange(0, 4)))
        else:
            chars = "".join(rng.choice(PS_WORDY + list(PS_SPACES) + PS_SPECIAL) for _ in range(rng.randrange(0, 4)))
            pad = lambda: "".join(rng.choice(chars + "x") for _ in range(rng.randrange(0, 4)))
        s = pad() + s + pad()
        return f"pystr {op} {h(s)} {'N' if chars is None else h(chars)}"
    if op == "exp":
        s = "".join(rng.choice(["\t", "\t", "a", "bc", "\n", "\r", "\r\n", "\x0b", "\u2028", "\u00e9", "\U0001f600", "  ", ps_atom()])
                    for _ in range(rng.randrange(0, 10)))
        return f"pystr exp {h(s)} {rng.choice([8, 8, 4, 1, 0, -1, 3, 16])}"
    if op in ("part", "rpart"):
        return f"pystr {op} {h(s)} {h(ps_sub(s)) if rng.random() < 0.95 else '-'}"
    if op in ("center", "ljust", "rjust"):
        fill = rng.choice([" ", " ", "*", "\u00e9", "\U0001f600", "-", "\u0301", "0", " ", "*", "ab", ""])
        return f"pystr {op} {h(s)} {rng.choice([len(s) + rng.randrange(-2, 8), rng.randrange(-3, 25)])} {h(fill)}"
    if op == "zfill":
        s = rng.choice(["", "+", "-", "+1", "-1", "12", "-0", "+-", "--1", "a", "＋1", "-é"]) + (s if rng.random() < 0.3 else "")
        return f"pystr zfill {h(s)} {rng.choice([len(s) + rng.randrange(-2, 6), rng.randrange(-2, 12)])}"
    if op in ("find", "rfind", "index", "rindex", "count", "startswith", "endswith"):
        sub = ps_sub(s)
        if op in ("startswith", "endswith") and s and rng.random() < 0.4:
            sub = s[:rng.randrange(0, len(s) + 1)] if op == "startswith" else s[rng.randrange(0, len(s) + 1):]
        return f"pystr {op} {h(s)} {h(sub)} {ps_bound()} {ps_bound()}"
    if op == "replace":
        return f"pystr replace {h(s)} {h(ps_sub(s))} {h(rng.choice(['', 'X', 'xyz', 'éé', ps_text(2)]))} {rng.choice([-1, -1, 0, 1, 2, 3, -7])}"
    return f"pystr {op} {h(s)}"


def pystr_lines(count):
    # Every code point's case mappings and properties, in ranges of 0x2000: the table against Python.
    lines = [f"pystr rg {lo:x} {lo + 0x2000:x}" for lo in range(0, 0x110000, 0x2000)]
    return lines + [pystr_line() for _ in range(count)]


family("pystr", "pystr", pystr_lines(int(os.environ.get("PYSTR_LINES", "10000"))), ["python3.13", os.path.join(LIB, "pystr/oracle.py")])


# --- run ------------------------------------------------------------------------------------------------

HEX_ANSWERS = {"pyipaddress"}


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
