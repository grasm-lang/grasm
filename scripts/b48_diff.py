"""B48's packages against the references they reproduce.

- Java's random number generators against OpenJDK 17: java.util.Random,
  SplittableRandom and Collections.shuffle, sequence for sequence.
- Double.toString and Float.toString against Java 17, on edge doubles and
  random bit patterns.
- String.format against Java 17 with Locale.US: flags, widths,
  precisions, argument indexes and every conversion, refusals included.
- DecimalFormat against Java 17, both its fast path and its general one,
  in all eight rounding modes, with patterns Java refuses among them.
- java.net.URI against Java 17: parsing with Java's error reasons and
  indexes, every getter, normalize, resolve, relativize, the quoting
  constructors, equals, compareTo and hashCode.
- MessageFormat and ChoiceFormat against Java 17.
- BigDecimal against Java 17: parsing, printing, arithmetic with and
  without a MathContext, division to a scale, rounding and scale
  operations, comparison and hashing.
"""
import os
import random
import shutil
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B48_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B48_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B48_LIB") or os.path.join(ROOT, "scripts/lib/b48")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b48")
harness = os.path.join(work, "b48test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b48test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stdout[-800:] + built.stderr[-1500:]); sys.exit(1)
JAVA = os.path.join(work, "java")
os.makedirs(JAVA)
for f in os.listdir(os.path.join(LIB, "java")):
    if f.endswith(".java"):
        shutil.copy(os.path.join(LIB, "java", f), JAVA)
subprocess.run(["javac", "-d", JAVA] + [os.path.join(JAVA, f) for f in os.listdir(JAVA) if f.endswith(".java")], check=True)

cases = []   # (family, op, path, want lines)


def case_file(text):
    p = os.path.join(work, f"c{len(cases)}_{rng.randrange(1 << 30)}.txt")
    open(p, "w", encoding="utf-8").write(text)
    return p


# --- Java's generators ----------------------------------------------------------------------------------

INT_BOUNDS = [1, 2, 3, 7, 10, 16, 100, 1 << 30, (1 << 30) + 1, 2147483647, 1000000007, 65536]
LONG_BOUNDS = [1, 2, 3, 1000, 1 << 40, (1 << 62) + 1, 9223372036854775807, 6]
RANGES = [(-5, 5), (0, 1), (-2147483648, 2147483647), (-2147483648, 0), (2147483600, 2147483647), (-1000000000, 2000000000), (10, 26),
          (-2147483648, -2147483647), (-1073741824, 1073741824)]


def java_line():
    seed = rng.choice([0, 1, 42, -1, 9223372036854775807, -9223372036854775808, rng.randrange(-(1 << 63), 1 << 63), rng.randrange(1000)])
    if rng.random() < 0.7:
        ops = []
        for _ in range(rng.randrange(1, 25)):
            k = rng.choice(["i", "ib", "ib", "ir", "l", "lb", "z", "f", "d", "g", "g", "b", "s"])
            if k == "ib":
                ops.append(f"ib:{rng.choice(INT_BOUNDS)}")
            elif k == "ir":
                a, b = rng.choice(RANGES)
                ops.append(f"ir:{a}:{b}")
            elif k == "lb":
                ops.append(f"lb:{rng.choice(LONG_BOUNDS)}")
            elif k == "b":
                ops.append(f"b:{rng.randrange(0, 11)}")
            elif k == "s":
                ops.append(f"s:{rng.randrange(0, 30)}")
            else:
                ops.append(k)
        return f"R {seed} " + " ".join(ops)
    ops = []
    for _ in range(rng.randrange(1, 25)):
        k = rng.choice(["i", "ib", "l", "z", "f", "d", "sp"])
        ops.append(f"ib:{rng.choice(INT_BOUNDS)}" if k == "ib" else k)
    return f"S {seed} " + " ".join(ops)


lines = [java_line() for _ in range(int(os.environ.get("JRAND_LINES", "2000")))]
want = subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "RandomOracle"], input="\n".join(lines) + "\n", capture_output=True, text=True).stdout.splitlines()
if len(want) != len(lines):
    print(f"Java answered {len(want)} of {len(lines)}"); sys.exit(1)
for k in range(0, len(lines), 200):
    cases.append(("jrand", "jrand", case_file("\n".join(lines[k:k + 200]) + "\n"), want[k:k + 200]))


# --- Double.toString and Float.toString, against Java 17 ------------------------------------------------

import struct


def dbits(v):
    return struct.unpack("<Q", struct.pack("<d", v))[0]


def double_value():
    r = rng.random()
    if r < 0.3:
        return struct.unpack("<d", struct.pack("<Q", rng.getrandbits(64)))[0]
    if r < 0.5:
        # Short decimals, where the digit-generation rule shows.
        return float(f"{rng.randrange(1, 10 ** rng.randrange(1, 18))}e{rng.randrange(-30, 30)}")
    if r < 0.6:
        return rng.choice([1e7, 9999999.999999998, 1e-3, 0.001 - 1e-19, 1e23, 2e23, 5e-324, 2.2250738585072014e-308, 1.7976931348623157e308,
                           4.9e-324, 1e21, 1e22, 0.1, 0.3, 2.0 ** 53, 2.0 ** 63, 2.0 ** 64, 123456789012345678.0, -0.0, 0.0])
    if r < 0.8:
        return rng.uniform(-1e4, 1e4) * 10 ** rng.randrange(-12, 12)
    return float(rng.randrange(-(1 << 62), 1 << 62))


def float_line():
    if rng.random() < 0.5:
        return f"f {rng.getrandbits(32):08x}"
    v = rng.choice([0.1, 0.2, 1e10, 3.4028235e38, 1.4e-45, 1e-3, 0.3, 1e7, 16777217.0, rng.uniform(-100, 100), 1e-40])
    return "f %08x" % struct.unpack("<I", struct.pack("<f", v))[0]


jd_lines = []
for _ in range(int(os.environ.get("JDOUBLE_LINES", "3000"))):
    if rng.random() < 0.8:
        v = double_value()
        if v != v:
            v = 1.0
        jd_lines.append(f"d {dbits(v):016x}")
    else:
        jd_lines.append(float_line())
jd_lines += ["d 7ff8000000000000", "d 7ff0000000000000", "d fff0000000000000", "f 7fc00000", "f ff800000", "f 80000000"]
want = subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "DoubleOracle"], input="\n".join(jd_lines) + "\n", capture_output=True, text=True).stdout.splitlines()
if len(want) != len(jd_lines):
    print(f"Java answered {len(want)} of {len(jd_lines)}"); sys.exit(1)
for k in range(0, len(jd_lines), 500):
    cases.append(("jdouble", "jdouble", case_file("\n".join(jd_lines[k:k + 500]) + "\n"), want[k:k + 500]))


# --- String.format, against Java 17 ------------------------------------------------------------------------

hx = lambda t: t.encode("utf-8").hex() or "-"


def spec():
    conv = rng.choice("dddxXooeEffffgGaAssSbBhHccn%" + "q" * 1)
    s = "%"
    r = rng.random()
    if r < 0.15:
        s += f"{rng.choice([1, 2, 3, 0, 9])}$"
    flags = "".join(rng.sample("-#+ 0,(<", rng.choice([0, 0, 0, 1, 1, 2, 3])))
    if rng.random() < 0.05:
        flags += "0"
    s += flags
    if rng.random() < 0.5:
        s += str(rng.choice([1, 3, 5, 8, 12, 20]))
    if rng.random() < 0.4:
        s += "." + str(rng.choice([0, 1, 2, 3, 6, 10, 15, 20]))
    return s + conv


def jarg():
    k = rng.choice("iilldddfSSSczbsn")
    if k == "i":
        return "i:" + str(rng.choice([0, 1, -1, 42, 2147483647, -2147483648, rng.randrange(-10 ** 9, 10 ** 9)]))
    if k == "l":
        return "l:" + str(rng.choice([0, -1, 9223372036854775807, -9223372036854775808, rng.randrange(-10 ** 15, 10 ** 15)]))
    if k == "b":
        return "b:" + str(rng.randrange(-128, 128))
    if k == "s":
        return "s:" + str(rng.randrange(-32768, 32768))
    if k == "d":
        v = rng.choice([0.0, -0.0, 1.005, 0.125, 2.5, 1e-5, 123456.789, 1e21, 1e-300, float("inf"), -float("inf"), float("nan"), 5e-324, 0.1, 9.995,
                        rng.uniform(-1e6, 1e6), rng.uniform(-1, 1) * 10 ** rng.randrange(-20, 20), 1.7976931348623157e308])
        return "d:%016x" % dbits(v)
    if k == "f":
        return "f:%08x" % struct.unpack("<I", struct.pack("<f", rng.choice([0.1, 1.5, -2.25, 3.4028235e38, 1e-40, rng.uniform(-100, 100)])))[0]
    if k == "S":
        return "S:" + hx(rng.choice(["", "hello", "Grüße", "a b", "ß", "x" * 30, "😀ok", "ıi"]))
    if k == "c":
        return "c:" + str(rng.choice([65, 97, 223, 8364, 0x3042]))
    if k == "z":
        return "z:" + rng.choice("tf")
    return "n"


# Flags each conversion accepts, and the arguments it takes.
VALID = {"d": ("-+ 0,(", "ilbs"), "o": ("-#0", "ilbs"), "x": ("-#0", "ilbs"), "X": ("-#0", "ilbs"), "e": ("-#+ 0(", "df"),
         "E": ("-#+ 0(", "df"), "f": ("-#+ 0,(", "df"), "g": ("-+ 0,(", "df"), "G": ("-+ 0,(", "df"), "a": ("-#+ 0", "df"),
         "A": ("-#+ 0", "df"), "s": ("-", "ildfSczbn"), "S": ("-", "ildfSczbn"), "b": ("-", "zSn"), "h": ("-", "ildfSczb"),
         "c": ("-", "icn")}


def valid_spec():
    conv = rng.choice(list(VALID))
    allowed, kinds = VALID[conv]
    flags = "".join(f for f in allowed if rng.random() < 0.25)
    if "+" in flags and " " in flags:
        flags = flags.replace(" ", "")
    if "-" in flags and "0" in flags:
        flags = flags.replace(rng.choice("-0"), "")
    width = ""
    if rng.random() < 0.6 or "-" in flags or "0" in flags:
        width = str(rng.choice([1, 4, 8, 12, 20]))
    prec = ""
    if conv not in "doxXc" and rng.random() < 0.5:
        prec = "." + str(rng.choice([0, 1, 2, 3, 6, 10, 15]))
    arg = jarg()
    while arg[0] not in kinds:
        arg = jarg()
    if conv in "oxX" and arg[0] in "ilbs" and ("+" in flags or " " in flags or "(" in flags):
        flags = flags.replace("+", "").replace(" ", "").replace("(", "")
    if conv == "c" and arg[0] == "i":
        arg = "i:" + str(rng.choice([65, 97, 0x1F600, 0x3042, 1114111]))
    return f"%{flags}{width}{prec}{conv}", arg


def format_line():
    pieces, args = [], []
    for _ in range(rng.randrange(1, 4)):
        if rng.random() < 0.4:
            pieces.append(rng.choice(["x=", " ", "[", "]", "é", "%%", "%n", "abc "]))
        if rng.random() < 0.8:
            sp, a = valid_spec()
            pieces.append(sp)
            args.append(a)
        else:
            pieces.append(spec())
            if rng.random() < 0.7:
                args.append(jarg())
    return hx("".join(pieces)) + (" " + " ".join(args) if args else "")


jf_lines = [format_line() for _ in range(int(os.environ.get("JFORMAT_LINES", "4000")))]
jf_lines += [hx(f) + " " + a for f, a in [("%.2f", "d:%016x" % dbits(1.005)), ("%.2f", "d:%016x" % dbits(0.125)), ("%00010d", "i:5"), ("%,d", "l:-1234567"),
                                         ("%x", "i:-1"), ("%x", "l:-1"), ("%h", "S:" + hx("hello")), ("%e", "d:0000000000000000"), ("%g", "d:%016x" % dbits(0.0001)),
                                         ("%a", "d:%016x" % dbits(1.0)), ("%.3a", "d:%016x" % dbits(0.1)), ("%s", "d:%016x" % dbits(1e23)), ("%-5c|", "i:65")]]
want = subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "FormatOracle"], input="\n".join(jf_lines) + "\n", capture_output=True, text=True).stdout.splitlines()
if len(want) != len(jf_lines):
    print(f"Java answered {len(want)} of {len(jf_lines)}"); sys.exit(1)
for k in range(0, len(jf_lines), 500):
    cases.append(("jformat", "jformat", case_file("\n".join(jf_lines[k:k + 500]) + "\n"), want[k:k + 500]))


# --- DecimalFormat, against Java 17 -------------------------------------------------------------------------

COMMON_PATTERNS = ["#,##0.###", "#,##0.00", "0.00", "#.##", "0.###E0", "##0.#####E0", "00.###E0", "#,##0.00;(#,##0.00)", "\u00a4#,##0.00",
                   "\u00a4\u00a4 #,##0.00", "#%", "0.0%", "#\u2030", "000000", "#", "", ".00", "#.", "0.", "#,#0", "0.0##", "##0.##E0",
                   "#,##0.###;-#", "'#'#", "0 'o''clock'", "#,##,###", "0.00E00", "#E0", "0.###E0;(0)", "#,##0.###%", "-#", "0;0E0"]
MODES = ["HALF_EVEN"] * 6 + ["HALF_UP", "HALF_DOWN", "UP", "DOWN", "CEILING", "FLOOR", "UNNECESSARY"]


def random_affix():
    return "".join(rng.choice(["", "", "x", "$", " ", "'#'", "''", "'a;b'", "%", "\u2030", "\u00a4", "-", "(", ")", "'%'", "kg", "E"])
                   for _ in range(rng.randrange(0, 3)))


def random_number_part():
    s = ""
    for _ in range(rng.randrange(0, 4)):
        s += rng.choice(["#", "#", ","])
    s += "0" * rng.randrange(0, 4)
    if rng.random() < 0.6:
        s += "." + "0" * rng.randrange(0, 4) + "#" * rng.randrange(0, 5)
    if rng.random() < 0.25:
        s += "E" + "0" * rng.randrange(0, 3)
    return s


def random_pattern():
    if rng.random() < 0.35:
        return rng.choice(COMMON_PATTERNS)
    p = random_affix() + random_number_part() + random_affix()
    if rng.random() < 0.2:
        p += ";" + random_affix() + random_number_part() + random_affix()
    return p


def decimal_value():
    if rng.random() < 0.25:
        return "l:" + str(rng.choice([0, 1, -1, 7, 1234567, -987654321, 10 ** 15, 9223372036854775807, -9223372036854775808,
                                      92233720368547758, rng.randrange(-10 ** 12, 10 ** 12)]))
    v = rng.choice([0.0, -0.0, 0.5, 1.5, 2.5, -2.5, 0.125, 0.375, 1.005, 2.675, 0.07, 1e-5, 0.0005, 0.00049, 123456.789, 1e21, 1e-300,
                    5e-324, float("inf"), -float("inf"), float("nan"), 0.1, 9.995, 999999.5, 1.7976931348623157e308, 1234.5,
                    rng.uniform(-1e6, 1e6), rng.uniform(-1, 1) * 10 ** rng.randrange(-12, 16), round(rng.uniform(-100, 100), rng.randrange(1, 4)),
                    rng.randrange(-10 ** 6, 10 ** 6) / 8, rng.randrange(1, 10 ** 4) / 1000 + 0.0005,
                    # Ties at the third and second place, where the two paths decide differently.
                    rng.randrange(0, 10 ** rng.randrange(1, 10)) + (2 * rng.randrange(0, 1000) + 1) / 2000,
                    rng.randrange(0, 10 ** rng.randrange(1, 10)) + (2 * rng.randrange(0, 100) + 1) / 200,
                    rng.choice([-1, 1]) * (2 * rng.randrange(0, 10 ** 6) + 1) / 2 ** rng.randrange(1, 12)])
    return "d:%016x" % dbits(v)


dec_lines = [f"{hx(random_pattern())} {rng.choice(MODES)} {decimal_value()}" for _ in range(int(os.environ.get("JDECIMAL_LINES", "6000")))]
want = subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "DecimalOracle"], input="\n".join(dec_lines) + "\n", capture_output=True, text=True).stdout.splitlines()
if len(want) != len(dec_lines):
    print(f"Java answered {len(want)} of {len(dec_lines)}"); sys.exit(1)
for k in range(0, len(dec_lines), 500):
    cases.append(("jdecimal", "jdecimal", case_file("\n".join(dec_lines[k:k + 500]) + "\n"), want[k:k + 500]))


# --- java.net.URI, against Java 17 --------------------------------------------------------------------------

ODD = [" ", "\"", "<", ">", "\\", "^", "`", "{", "|", "}", "%", "%4", "%zz", "%41", "%e9", "%C3%A9", "%ED%A0%80", "%F0%9F", "%2F", "[", "]",
       "\u00e9", "\u00a0", "\u0080", "\u0081", "\u2028", "\U0001F600", "e\u0301", "\u3000", "\x7f", "\t", "#", "?", "@", ":", ";", "=", "&",
       "$", "+", ",", "!", "*", "'", "(", ")", "~", "_", "-", "."]
SCHEMES = ["http", "HTTPS", "mailto", "file", "urn", "a+b.c-d", "x", "jar", "1x", "", "h_t"]
HOSTS = ["a", "example.com", "EXAMPLE.org", "a-b.c", "a.b.c.", "-a", "a-", "a_b", "1a.b", "a.1b", "1.2.3.4", "1.2.3", "256.1.1.1",
         "01.2.3.4", "1.2.3.4.5", "999999999999.1.1.1", "[::1]", "[::]", "[1:2:3:4:5:6:7:8]", "[1:2:3:4:5:6:7:8:9]", "[1:2:3:4:5:6:7]",
         "[::ffff:1.2.3.4]", "[fe80::1%en0]", "[fe80::1%]", "[fe80::1%25en0]", "[1::2::3]", "[12345::]", "[::1.2.3]", "[::g]", "[1.2.3.4]",
         "[::1", "[]", "[::1]x", "[::99999999999.1.1.1]", "h\u00e9", "%41", "xn--n3h", "a..b", ".a"]


def odd():
    return rng.choice(ODD)


def segment():
    parts = [rng.choice(["a", "b", "c", "..", ".", "", "g", "x;p", "a:b", "%2e", ".g", "g.", "..g", "~u"])]
    if rng.random() < 0.2:
        parts.append(odd())
    return "".join(parts)


def random_uri():
    if rng.random() < 0.1:
        return "".join(odd() if rng.random() < 0.5 else rng.choice("ab:/?#@.") for _ in range(rng.randrange(0, 8)))
    u = ""
    if rng.random() < 0.7:
        u += rng.choice(SCHEMES) + ":"
    if rng.random() < 0.6:
        u += "//"
        if rng.random() < 0.25:
            u += rng.choice(["u", "u:p", "", "u@v", "%75", "u\u00e9", "u p"]) + "@"
        if rng.random() < 0.9:
            u += rng.choice(HOSTS)
        if rng.random() < 0.3:
            u += ":" + rng.choice(["80", "", "0", "65536", "2147483647", "2147483648", "8a", "-1"])
    elif rng.random() < 0.3 and u:
        u += rng.choice(["a@b", "x", "//[::1", "%41b", "\u00e9", ""])
    path = "/".join(segment() for _ in range(rng.randrange(0, 5)))
    if rng.random() < 0.6:
        path = "/" + path
    u += path
    if rng.random() < 0.3:
        u += "?" + rng.choice(["", "a=b", "a=b&c", "%e9", "x y", "[x]", "q?r", "a/../b"]) + (odd() if rng.random() < 0.2 else "")
    if rng.random() < 0.25:
        u += "#" + rng.choice(["", "f", "a#b", "%41", "s/../x", "\u00e9"]) + (odd() if rng.random() < 0.2 else "")
    return u


RFC_BASE = "http://a/b/c/d;p?q"
RFC_CHILDREN = ["g:h", "g", "./g", "g/", "/g", "//g", "?y", "g?y", "#s", "g#s", "g?y#s", ";x", "g;x", "g;x?y#s", "", ".", "./", "..", "../",
                "../g", "../..", "../../", "../../g", "../../../g", "../../../../g", "/./g", "/../g", "g.", ".g", "g..", "..g", "./../g", "./g/.",
                "g/./h", "g/../h", "g;x=1/./y", "g;x=1/../y", "g?y/./x", "g?y/../x", "g#s/./x", "g#s/../x", "http:g", "a:b", "./a:b", "#", "?"]


def ux(t):
    return hx(t)


def opt(t):
    return "~" if t is None else ux(t)


def uri_line():
    r = rng.random()
    if r < 0.4:
        return "p " + ux(random_uri())
    if r < 0.45:
        return "a " + ux(random_uri())
    if r < 0.65:
        base = rng.choice([RFC_BASE, "http://a/b/c/d;p?q#f", "file:///x/y", "a/b/c", "/a/b", "mailto:x", "http://h", "http://h/", "x:y/z", random_uri()])
        child = rng.choice(RFC_CHILDREN + [random_uri()])
        return f"r {ux(base)} {ux(child)}"
    if r < 0.75:
        base = rng.choice(["http://a/b/", "http://a/b", "HTTP://A/b/", "http://a/b/../b/", "file:/x/", "/a/", "a/", random_uri()])
        child = rng.choice(["http://a/b/c", "http://A/b/c?q#f", "http://a/bc", "http://a/b", "file:/x/y", "/a/b/c", "a/b", "http://a/b/./c", random_uri()])
        return f"l {ux(base)} {ux(child)}"
    if r < 0.85:
        a = random_uri()
        b = rng.choice([a, a.upper(), a.replace("%e9", "%E9"), random_uri(), a + "x"])
        return f"q {ux(a)} {ux(b)}"
    pick = lambda xs: rng.choice(xs + [None])
    if r < 0.93:
        return "c7 " + " ".join([opt(pick(["http", "file", "x"])), opt(pick(["u", "u p", "a@b", "\u00e9", "%41"])), opt(pick(["h", "::1", "[::1]", "a b", "1.2.3.4", "h\u00e9"])),
                                 str(rng.choice([-1, 80, 0])), opt(pick(["/a b", "/x%y", "rel", "", "/\u00e9\u00a0", "/[x]"])), opt(pick(["a=b c", "%", "\u3000"])),
                                 opt(pick(["f g", "#", "\u0080"]))])
    if r < 0.97:
        return "c5 " + " ".join([opt(pick(["http", "x"])), opt(pick(["h:80", "u@h", "a b", "[::1]:8", "[::1", "[x]y z", "%zz"])), opt(pick(["/p q", "", "rel"])),
                                 opt(pick(["q r"])), opt(pick(["f"]))])
    return "c3 " + " ".join([opt(pick(["mailto", "urn", "http"])), opt(pick(["a b@c", "//h/p q", "//[::1]/x y", "//[x", "", "\u00e9 \u2028"])), opt(pick(["f g"]))])


uri_lines = [uri_line() for _ in range(int(os.environ.get("JURI_LINES", "6000")))]
uri_lines += [f"r {ux(RFC_BASE)} {ux(c)}" for c in RFC_CHILDREN]
want = subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "UriOracle"], input="\n".join(uri_lines) + "\n", capture_output=True, text=True).stdout.splitlines()
if len(want) != len(uri_lines):
    print(f"Java answered {len(want)} of {len(uri_lines)}"); sys.exit(1)
for k in range(0, len(uri_lines), 500):
    cases.append(("juri", "juri", case_file("\n".join(uri_lines[k:k + 500]) + "\n"), want[k:k + 500]))


# --- MessageFormat and ChoiceFormat, against Java 17 -------------------------------------------------------

def choice_text():
    out = []
    limit = rng.choice([-1, 0, 0, 1])
    for k in range(rng.randrange(0, 4)):
        lim = rng.choice([str(limit)] * 6 + [str(limit) + ".0", "\u221e", "-\u221e", " " + str(limit), str(limit) + "d", "+" + str(limit), "1e0"]
                         + (["x", ""] if rng.random() < 0.1 else []))
        sep = rng.choice("##<\u2264")
        body = rng.choice(["none", "one", "many", "{0}", "{0,number,integer} items", "''q''", "'{'0'}'", "a|b", "{1}", "{0,choice,0#z|1#{0}}", ""])
        out.append(lim + sep + body)
        limit += rng.choice([1, 1, 2, 0])
    return "|".join(out)


def placeholder():
    idx = rng.choice(["0"] * 6 + ["1"] * 4 + ["2", "5"] if rng.random() < 0.9 else ["-1", "x", "", " 0", "+1", "00", "99999999999"])
    r = rng.random()
    if r < 0.3:
        return "{" + idx + "}"
    kind = rng.choice(["number"] * 4 + ["choice"] * 4 + ["NUMBER", " number", "number ", "choice "] + (["bogus", "", " "] if rng.random() < 0.2 else []))
    if kind.strip().lower() == "choice":
        return "{" + idx + "," + kind + "," + choice_text() + "}"
    style = rng.choice(["", "integer", "currency", "percent", " Integer ", "#,##0.00", "0.###E0", "'#'#", "#.#.#", "00%", "\u00a4#"])
    return "{" + idx + "," + kind + ("," + style if rng.random() < 0.8 else "") + "}"


def message_pattern():
    pieces = []
    for _ in range(rng.randrange(1, 5)):
        pieces.append(rng.choice(["", "x ", "it''s ", "''", "'{0}'", "a'b'c", "\u00e9 ", " "] * 3 + ["'", "}", "{", "it's "]))
        pieces.append(placeholder())
    return "".join(pieces)


def message_arg():
    k = rng.choice("lllddsn")
    if k == "l":
        return "l:" + str(rng.choice([0, 1, 2, 3, -1, 42, 1234567, -9223372036854775808, 9223372036854775807]))
    if k == "d":
        return "d:%016x" % dbits(rng.choice([0.0, -0.0, 0.5, 1.0, 1.5, 2.0, 0.125, 1234.5678, -3.25, float("nan"), float("inf"), 1e-7, 0.0005]))
    if k == "s":
        return "s:" + hx(rng.choice(["", "text", "{0}", "\u00e9", "'q'"]))
    return "n"


msg_lines = [hx(message_pattern()) + " " + " ".join(message_arg() for _ in range(rng.randrange(0, 4))) for _ in range(int(os.environ.get("JMESSAGE_LINES", "5000")))]
msg_lines = [l.rstrip() for l in msg_lines]
want = subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "MessageOracle"], input="\n".join(msg_lines) + "\n", capture_output=True, text=True).stdout.splitlines()
if len(want) != len(msg_lines):
    print(f"Java answered {len(want)} of {len(msg_lines)}"); sys.exit(1)
for k in range(0, len(msg_lines), 500):
    cases.append(("jmessage", "jmessage", case_file("\n".join(msg_lines[k:k + 500]) + "\n"), want[k:k + 500]))


# --- BigDecimal, against Java 17 ----------------------------------------------------------------------------

BD_MODES = ["UP", "DOWN", "CEILING", "FLOOR", "HALF_UP", "HALF_DOWN", "HALF_EVEN", "UNNECESSARY"]


def bd_text():
    r = rng.random()
    if r < 0.15:
        return rng.choice(["0", "0.00", "-0", "0E+3", "0E-10", "1", "-1", "5", "0.5", "2.5", "-2.5", "1.005", "123.4500", "-9.99", "0.000001",
                           "0.0000001", "1E+3", "1e-7", "1E10", "9.999", "99.95", "100", "1000", "3", "7", "0.1", "1.0", "10", "1E+1"])
    if r < 0.18:
        return rng.choice(["", ".", "1e", "abc", "1..2", "+-1", " 1", "1e+", "e5", "1.2.3", "--1", "1E2147483648", "1E-2147483649"])
    digits = "".join(rng.choice("0123456789") for _ in range(rng.randrange(1, rng.choice([4, 8, 20, 40]))))
    if rng.random() < 0.5:
        k = rng.randrange(0, len(digits) + 1)
        digits = digits[:k] + "." + digits[k:]
        if digits == ".":
            digits = "0."
    if rng.random() < 0.3:
        digits += rng.choice("eE") + rng.choice(["", "+", "-"]) + str(rng.randrange(0, 25))
    return rng.choice(["", "", "-", "+"]) + digits


def bd_line():
    r = rng.random()
    a, b = hx(bd_text()), hx(bd_text())
    if r < 0.12:
        return "s " + a
    if r < 0.17:
        v = rng.choice([0.1, -0.0, 0.0, 1.5, 1e-7, 1e21, 123.456, 5e-324, 1.7976931348623157e308, float("nan"), float("inf"), rng.uniform(-1e6, 1e6)])
        return rng.choice(["nbd ", "vod "]) + "%016x" % dbits(v)
    if r < 0.45:
        return rng.choice(["add", "sub", "mul", "div", "dti", "rem", "cmp", "cmp"]) + f" {a} {b}"
    if r < 0.6:
        return rng.choice(["divs"]) + f" {a} {b} {rng.randrange(-5, 15)} {rng.choice(BD_MODES)}" if rng.random() < 0.7 else f"divm {a} {b} {rng.choice(BD_MODES)}"
    if r < 0.85:
        return rng.choice(["addc", "subc", "mulc", "divc", "divc"]) + f" {a} {b} {rng.choice([0, 1, 2, 3, 5, 7, 10, 16, 34])} {rng.choice(BD_MODES)}"
    if r < 0.9:
        return f"round {a} {rng.choice([0, 1, 2, 3, 5, 9])} {rng.choice(BD_MODES)}"
    if r < 0.95:
        return f"sets {a} {rng.randrange(-4, 12)} {rng.choice(BD_MODES)}"
    return rng.choice(["pow", "mpl", "mpr", "sbp"]) + f" {a} {rng.choice([0, 1, 2, 3, 5, -2, 7])}"


bd_lines = [bd_line() for _ in range(int(os.environ.get("JBIG_LINES", "6000")))]
want = subprocess.run(["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "BigOracle"], input="\n".join(bd_lines) + "\n", capture_output=True, text=True).stdout.splitlines()
if len(want) != len(bd_lines):
    print(f"Java answered {len(want)} of {len(bd_lines)}"); sys.exit(1)
for k in range(0, len(bd_lines), 500):
    cases.append(("jbig", "jbig", case_file("\n".join(bd_lines[k:k + 500]) + "\n"), want[k:k + 500]))


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
