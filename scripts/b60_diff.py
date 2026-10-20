"""B60's packages against the references they reproduce.

- Perl 5.34's sprintf: every conversion and flag, vectors, explicit
  indexes, numification of string arguments, and the warnings and deaths.
- Perl 5.34's Text::Abbrev: abbrev over word lists with shared
  truncations and repeats.
- MIME::Base64 3.16 (Perl 5.34): encode_base64 with its eol, the lenient
  decode_base64, both lengths, the URL forms, and MIME::QuotedPrint's
  encode_qp (eol, binary) and decode_qp.
- Getopt::Long 2.52 (Perl 5.34): GetOptionsFromArray into a hash, under
  generated configurations, specs and command lines.
- Text::Balanced 2.04 (Perl 5.34): extract_delimited and extract_bracketed
  with their delimiters, escapes and prefix patterns, and the errors.
- SBCL 2.6.8's FORMAT for numbers: ~F ~E ~G ~$ and ~D ~B ~O ~X with their
  parameters and modifiers, over integers, ratios, singles and doubles.
- SBCL 2.6.8's reader on number tokens: read-from-string under every
  *read-base* and *read-default-float-format*.
- Rust 1.99's format specs ({:spec}) over i64, u64, f64 and str, through
  generated format! calls compiled by rustc.
- Lua 5.5's arithmetic: integer and float subtypes through every operator,
  string coercion, comparisons, math functions and tonumber with a base.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B60_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B60_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B60_LIB") or os.path.join(ROOT, "scripts/lib/b60")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b60")
harness = os.path.join(work, "b60test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b60test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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
    if not lines:
        return
    want = subprocess.run(oracle_argv, input="\n".join(lines) + "\n", capture_output=True, text=True).stdout.splitlines()
    if len(want) != len(lines):
        print(f"{name}: the oracle answered {len(want)} of {len(lines)}"); sys.exit(1)
    for k in range(0, len(lines), chunk):
        cases.append((name, op, case_file("\n".join(lines[k:k + chunk]) + "\n"), want[k:k + chunk]))


# --- Perl's sprintf, against Perl 5.34 ------------------------------------------------------------------

PS_STRS = ["0", "1", "7", "12", "-12", "+3", "255", "65", "233", "9786", "1114111", "1114112", "55296", "-1", "42",
           "9223372036854775807", "9223372036854775808", "18446744073709551615", "18446744073709551616", "-9223372036854775808",
           "-9223372036854775809", "-10000000000000000000", "99999999999999999999", "9007199254740993", "4503599627370495.9999999",
           "1.5", "-2.5", "0.5", "2.5", "1e3", "1e19", "1e20", "1e400", "-1e400", "1e-400", ".5", "5.", "0.1", "-0", "-0.0", "3.7", "-3.7",
           "1.5e300", "123456789.987654321", "1e15", "1e16", "6.02e23",
           " 12 ", "\t7\n", "  ", " ", "", "12abc", "abc", "0x1A", "0b101", "1_000", "--1", "+-inf", "-+inf", "- 5", "+ 0x10", "+ 0x1p3",
           "- 0X1.8p1", "0 but true", "0 but false", "1e", "1e+", ".", "+", "-", "12\x00", "12 x", "1 2", "\x0b5", "5\x0c",
           "inf", "Inf", "-inf", "+inf", "infinity", "INFINITY", "-Infinity", "infx", "infinit", "info", " inf", " -inf", "inf ", "inf\x00x",
           "nan", "NaN", "-nan", "NAN", "nanq", "nans", "nan(123)", "nan(12x)", "nan(1.5)", "nan(0x1f)", "nan(0b101)", "nan(", "nan()",
           "nan(1_2)", "nanx", " nan", "1.#INF", "1.#IND", "-1.#INF", "1#INF", " 1.#INF", "1.#INF00", "1.#IND00x", "Infinityx", "+ inf",
           "- nan", "1.#", "1.#Q", "é", "☺", "日本", "á", "é12", "12é", "a\nb\tc\\d\x7f\x01", "\x00", "x" * 60, "y" * 56, "z" * 55,
           "☺" * 20, "é" * 33, "1.2.3", "1.22.333", "v1.2", "A", "AB", "\x7f\x80", "€", "\U0001f600", "-0b101", "-0x1A",
           "+0b1", "-0B11", "-0X10"]


def ps_str():
    r = rng.random()
    if r < 0.6:
        return rng.choice(PS_STRS)
    if r < 0.75:
        return str(rng.randrange(-10 ** rng.randrange(1, 22), 10 ** rng.randrange(1, 22)))
    if r < 0.85:
        return repr(rng.uniform(-1e6, 1e6)) if rng.random() < 0.5 else f"{rng.uniform(-10, 10):.{rng.randrange(0, 8)}e}"
    return "".join(rng.choice(["1", "2", "9", "0", ".", "e", "-", "+", " ", "a", "i", "n", "f", "x", "\t", "é", "#"]) for _ in range(rng.randrange(0, 8)))


PS_INTS = [0, 1, -1, 7, 42, 65, 97, 127, 128, 233, 255, 256, 9786, 55296, 57343, 57344, 65535, 65536, 70000, 1114111, 1114112, 2 ** 31,
           2 ** 31 - 1, -2 ** 31, 2 ** 53 + 1, 2 ** 62, 2 ** 63 - 1, -2 ** 63, -2 ** 63 + 1, 300, -300, 32767, 32768, -32769, 1000000, 10 ** 15]


def ps_int():
    r = rng.random()
    if r < 0.5:
        return rng.choice(PS_INTS)
    if r < 0.8:
        return rng.randrange(-1000, 100000)
    return rng.randrange(-2 ** 63, 2 ** 63)


PS_NUMS = [0.0, -0.0, 0.5, 1.5, 2.5, -0.5, -2.5, 0.125, 0.375, 1.0, -1.0, 3.7, -3.7, 65.7, 1e15, 1e16, 1e19, 1e20, 1.5e300, -1.5e300, 1e300,
           5e-324, 1e-323, 2.2250738585072014e-308, 2.225073858507201e-308, 1.7976931348623157e308, float("inf"), float("-inf"), float("nan"),
           0.1, 1 / 3, 123456.789, 999999.5, 0.9999995, 9.9999995, 99.5, 1e-5, 1.25e-5, 0.0001, 0.00001, 123456789012345678.0, 9.5, 10.5,
           2 ** 63, 2 ** 64, -2 ** 63, 2 ** 53 + 2.0, 0.30000000000000004, 1.0000000000000002, 33.0, 9786.0, 4.35, 2.675, 1e21, 1e-7]


def ps_num():
    r = rng.random()
    if r < 0.45:
        return rng.choice(PS_NUMS)
    if r < 0.6:
        return rng.randrange(-10 ** 6, 10 ** 6) / rng.choice([1, 2, 4, 8, 16, 10, 100, 1000])
    if r < 0.8:
        return rng.uniform(-1, 1) * 10.0 ** rng.randrange(-30, 30)
    if r < 0.9:
        return struct.unpack("<d", struct.pack("<Q", rng.randrange(0, 1 << 52) | (rng.choice([0, 0, 1, 2046]) << 52) | (rng.randrange(2) << 63)))[0]
    v = struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]
    return v


def ps_arg(kind=None):
    k = kind or rng.choice("ssiinn")
    if k == "s":
        return "s" + hx(ps_str())
    if k == "i":
        return "i" + str(ps_int())
    return "n" + "%016x" % dbits(ps_num())


PS_LETTERS = "csdiuoxXbBeEfFgGaADUO%"
PS_BAD = "yzkwPrHIJmY!@&~é\x00"
PS_LITS = ["", "", "a", " ", "|", "x=", "é", "☺", "\t", "[", "]", "-", "0", "1", "v", "*", "$", ".", "abc", "日", "\x01"]


def ps_number(small=True):
    r = rng.random()
    if r < 0.6:
        return str(rng.randrange(1, 12))
    if r < 0.85:
        return str(rng.randrange(1, 40))
    if r < 0.95 or small:
        return str(rng.randrange(40, 70))
    return rng.choice(["99999999999999999999", "4611686018427387903", "4611686018427387904", "9223372036854775807"])


def ps_directive():
    """A conversion, and how many arguments it reads in order (explicit
    indexes aside)."""
    d, need = "%", 0
    if rng.random() < 0.15:
        d += str(rng.choice([1, 1, 2, 3, 4, 5])) + "$"
    for _ in range(rng.choice([0, 0, 1, 1, 2, 3])):
        d += rng.choice("-+ 0#")
    if rng.random() < 0.12:
        v = rng.choice(["v", "v", "*v", "*2$v", "*1$v"])
        d += v
        need += 1 if v == "*v" else 0
        if rng.random() < 0.3:
            d += "0"
    r = rng.random()
    if r < 0.35:
        d += ps_number(rng.random() < 0.97)
    elif r < 0.45:
        d += "*"
        need += 1
    elif r < 0.48:
        d += "*" + str(rng.randrange(1, 5)) + "$"
    r = rng.random()
    if r < 0.3:
        d += "." + rng.choice(["", "0", "00", "1", "2", "3", "5", "6", "10", "15", "17", "20", "30", "40", "01"])
    elif r < 0.38:
        d += ".*"
        need += 1
    elif r < 0.4:
        d += ".*" + str(rng.randrange(1, 5)) + "$"
    if rng.random() < 0.15:
        d += rng.choice(["h", "hh", "l", "ll", "q", "L", "V", "z", "t", "j"])
    r = rng.random()
    if r < 0.93:
        c = rng.choice(PS_LETTERS)
    elif r < 0.98:
        c = rng.choice(PS_BAD)
    else:
        c = ""
    if c not in ("%", "") and c not in PS_BAD:
        need += 1
    return d + c, need


# Arguments that may become a width or precision: small, or large enough
# that Perl dies rather than allocating terabytes.
PS_SAFE_STRS = ["0", "1", "7", "12", "-12", "+3", "-1", "42", "255", " 12 ", "\t7\n", "", "12abc", "abc", "inf", "-inf", "nan", " inf", "infx",
                "1e3", "-0", "0.5", "3.7", "-3.7", "1e400", "-1e400", "1e19", "18446744073709551615", "18446744073709551616",
                "9999999999999999999", "9223372036854775807", "9223372036854775808", "-9223372036854775808", "-9223372036854775809",
                "4611686018427387904", "-4611686018427387904", "99999999999999999999", "0 but true", "--1", "- 5", "1_000", "é", "☺",
                " 1.#INF", "nan(1.5)", "1e", "+ 0x10", "1.#INF", "65", "233", "9786"]
PS_SAFE_INTS = [0, 1, -1, 5, -5, 12, 30, -30, 255, 1000, -1000, 2 ** 62, -2 ** 62, 2 ** 63 - 1, -2 ** 63, 65, 9786]
PS_SAFE_NUMS = [0.0, -0.0, 1.5, -1.5, 12.7, -12.7, 99.99, 1e19, 1e20, -1e19, float("inf"), float("-inf"), float("nan"), 2.0 ** 62, -(2.0 ** 62),
                1e300, -1e300, 5e-324, 3.0, 0.5]


def ps_safe_arg():
    k = rng.choice("ssiinn")
    if k == "s":
        return "s" + hx(rng.choice(PS_SAFE_STRS) if rng.random() < 0.8 else str(rng.randrange(-100, 100)))
    if k == "i":
        return "i" + str(rng.choice(PS_SAFE_INTS) if rng.random() < 0.6 else rng.randrange(-100, 100))
    return "n" + "%016x" % dbits(rng.choice(PS_SAFE_NUMS) if rng.random() < 0.6 else rng.uniform(-100, 100))


def ps_arg_for(fmt):
    if "*" in fmt:
        return ps_safe_arg()
    last = fmt[-1:] if fmt else ""
    if last in "eEfFgGaA":
        return ps_arg(rng.choice("nnnss i"[:6]))
    if last == "c":
        r = rng.random()
        if r < 0.5:
            return "i" + str(rng.choice([65, 97, 0, 10, 127, 128, 200, 233, 255, 256, 9786, 55296, 57343, 1114111, 1114112, 2 ** 40, -1,
                                         2 ** 63 - 1, rng.randrange(0, 0x3000)]))
        return ps_arg()
    if last == "s" or "v" in fmt:
        return ps_arg(rng.choice("ssssin"))
    return ps_arg()


def ps_hexy_double():
    # Nybbles rich in 8s and 0s, so that %a's one-digit tie rule shows.
    frac = 0
    for _ in range(13):
        frac = frac * 16 + rng.choice([0, 0, 8, 8, 8, 1, 7, 9, 15, rng.randrange(16)])
    exp = rng.choice([1023, 1023, 1022, 1030, 1000, 0, 0, 1, 2046, rng.randrange(2047)])
    return struct.unpack("<d", struct.pack("<Q", (rng.randrange(2) << 63) | (exp << 52) | frac))[0]


def perlsprintf_line():
    r = rng.random()
    if r < 0.05:
        fmt = "%" + "".join(rng.choice("-+ 0#") for _ in range(rng.choice([0, 0, 1, 2]))) + rng.choice(["", "", "12", "25"]) + \
            "." + str(rng.randrange(0, 15)) + rng.choice("aA")
        return f"{hx(fmt)} n{dbits(ps_hexy_double()):016x}"
    if r < 0.08:
        fmt = rng.choice(["é", "☺|", "%s", "", "x"]) + "%" + rng.choice(["3", "-3", "4", "03", "2", "1", ".1", ".0", "-5"]) + "c|" + rng.choice(["", "%c"])
        args = (["s" + hx(rng.choice(["é", "a", "☺"]))] if "%s" in fmt else []) + \
            ["i" + str(rng.choice([128, 200, 233, 255, 65, 256, 9786, 127])) for _ in range(fmt.count("%") - fmt.count("%s"))]
        return " ".join([hx(fmt)] + args)
    if r < 0.18:
        # Mangled formats, for the invalid-conversion path.
        fmt = "".join(rng.choice(["%", "%", "-", "+", " ", "0", "#", "*", "v", ".", "1", "2", "$", "h", "l", "q", "d", "s", "x", "y", "é", "\x00",
                                  "a", "z", "*1$", "*v", "vv"]) for _ in range(rng.randrange(1, 9)))
        return " ".join([hx(fmt)] + [ps_arg_for(fmt) for _ in range(rng.randrange(0, 4))])
    if r < 0.32:
        # One argument read several ways, so numification is shared.
        k = rng.choice("ssssin")
        uses = [rng.choice(["%1$d", "%1$s", "%1$f", "%1$g", "%1$u", "%1$x", "%1$c", "%1$*1$d|", "%1$.*1$f", "%1$e", "%1$vd", "%1$i", "%1$.0f",
                            "%1$#o", "%1$hd", "%1$a"]) for _ in range(rng.randrange(2, 4))]
        fmt = " ".join(uses)
        a = ps_safe_arg() if "*" in fmt else ps_arg(k)
        return " ".join([hx(fmt)] + [a] + ([ps_arg_for(fmt)] if rng.random() < 0.2 else []))
    pieces, need = [], 0
    for _ in range(rng.choice([1, 1, 1, 2, 2, 3, 4])):
        pieces.append(rng.choice(PS_LITS))
        d, n = ps_directive()
        pieces.append(d)
        need += n
    pieces.append(rng.choice(PS_LITS))
    fmt = "".join(pieces)
    count = need + rng.choice([0, 0, 0, 0, 0, 0, 1, -1, 2])
    args = [ps_arg_for(fmt) for _ in range(max(count, 0))]
    return " ".join([hx(fmt)] + args)


family("perlsprintf", "perlsprintf", [perlsprintf_line() for _ in range(int(os.environ.get("PERLSPRINTF_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perlsprintf/oracle.pl")])


# --- Perl's Text::Abbrev, against Perl 5.34 ----------------------------------------------------------------

TA_STEMS = ["list", "lis", "li", "l", "edit", "ed", "send", "abort", "abo", "gripe", "grip", "a", "ab", "abc", "abd", "abe", "abcd", "", "x",
            "é", "éa", "éb", "日本", "日本語", "日", "a\u0301", "a\u0301b", "☺☺", "☺", "1", "12", "123", "10", " ", " a", "a ", "A", "Ab"]


def ta_word():
    r = rng.random()
    if r < 0.5:
        return rng.choice(TA_STEMS)
    if r < 0.8:
        return rng.choice(TA_STEMS) + "".join(rng.choice("abcé日 ") for _ in range(rng.randrange(0, 3)))
    return "".join(rng.choice("aabbcé") for _ in range(rng.randrange(0, 6)))


def perltextabbrev_line():
    n = rng.choice([0, 1, 1, 2, 2, 3, 3, 4, 5, 6, 8])
    words = [ta_word() for _ in range(n)]
    if words and rng.random() < 0.2:
        words.append(rng.choice(words))
    rng.shuffle(words)
    return " ".join(hx(w) for w in words) if words else "~"


family("perltextabbrev", "perltextabbrev", [perltextabbrev_line() for _ in range(int(os.environ.get("PERLTEXTABBREV_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perltextabbrev/oracle.pl")])


# --- Perl's MIME::Base64 and MIME::QuotedPrint, against Perl 5.34 --------------------------------------

MB_EOLS = ["~", "~", "-", "0d0a", "0a", "2a2a", "e282ac", "00"]


def mb_data():
    n = rng.choice([0, 1, 2, 3, 4, 5, 56, 57, 58, 59, 113, 114, 115, rng.randrange(0, 130)])
    return bytes(rng.randrange(256) for _ in range(n))


def mb_b64_text(url=False):
    import base64
    t = list((base64.urlsafe_b64encode if url else base64.b64encode)(mb_data()))
    if rng.random() < 0.3:
        t = t[:rng.randrange(len(t) + 1)]
    for _ in range(rng.choice([0, 0, 1, 2, 4])):
        i = rng.randrange(len(t) + 1)
        k = rng.random()
        if k < 0.45:
            t.insert(i, rng.choice(b" \n\r\t*-_.=\x80\xff\x00+/A0"))
        elif k < 0.7 and t:
            del t[min(i, len(t) - 1)]
        elif t:
            t[min(i, len(t) - 1)] = rng.choice(b"=A/+_-")
    return bytes(t)


QP_PIECES = [b"a", b"hello", b" ", b"\t", b"  ", b"\n", b"\r\n", b"\r", b"=", b"_", b".", b"\x00", b"\xe9", b"\xff", b"\x7f", b"x" * 70,
             b"y" * 74, b"z" * 75, b"w" * 73, b"v" * 72, b"u" * 76, b"=41", b"\x0c", b" \n", b"\t\n", b"~", b"\x01"]


def mb_qp_data():
    return b"".join(rng.choice(QP_PIECES) for _ in range(rng.randrange(0, 10)))


def mb_qp_text():
    import quopri
    t = list(quopri.encodestring(mb_qp_data()))
    for _ in range(rng.choice([0, 1, 2, 3])):
        i = rng.randrange(len(t) + 1)
        piece = rng.choice([b"=\n", b"= \r\n", b"=\t\n", b"=4", b"=g1", b"=41", b"=e9", b"=E9", b" \n", b"\t\r\n", b"\r\n", b"=", b"  ", b"=\r",
                            b"= x", b"\r", b"==41"])
        t[i:i] = list(piece)
    return bytes(t)


def perlmimebase64_line():
    h = lambda b: b.hex() or "-"
    r = rng.random()
    if r < 0.2:
        return f"e64 {h(mb_data())} {rng.choice(MB_EOLS)}"
    if r < 0.4:
        return f"d64 {h(mb_b64_text())}"
    if r < 0.45:
        return f"l64 {h(mb_data())} {rng.choice(MB_EOLS)}"
    if r < 0.5:
        return f"dl64 {h(mb_b64_text())}"
    if r < 0.55:
        return f"eu {h(mb_data())}"
    if r < 0.6:
        return f"du {h(mb_b64_text(True))}"
    if r < 0.82:
        return f"eqp {h(mb_qp_data())} {rng.choice(MB_EOLS)} {rng.choice(['~', '~', '0', '1', '00'])}"
    return f"dqp {h(mb_qp_text())}"


family("perlmimebase64", "perlmimebase64", [perlmimebase64_line() for _ in range(int(os.environ.get("PERLMIMEBASE64_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perlmimebase64/oracle.pl")])


# --- Perl's Getopt::Long, against Perl 5.34 --------------------------------------------------------------

GL_CONFIGS = [[], [], ["bundling"], ["bundling"], ["bundling_override"], ["bundling_values"], ["gnu_getopt"], ["gnu_compat"], ["no_ignore_case"],
              ["ignore_case_always"], ["no_auto_abbrev"], ["require_order"], ["pass_through"], ["no_getopt_compat"], ["posix_default"],
              ["permute"], ["no_bundling"], ["NoIgnoreCase"], ["default"], ["bundling", "no_ignore_case"], ["bundling", "pass_through"],
              ["require_order", "pass_through"], ["gnu_getopt", "no_ignore_case"], ["bundling", "ignore_case_always"], ["posix_default", "bundling"],
              ["no_getopt_compat", "bundling_override"], ["pass_through", "no_auto_abbrev"], ["Bundling", "PERMUTE"], ["gnu_compat", "pass_through"],
              ["bundling_values", "no_ignore_case"], ["posix_default", "pass_through"]]
GL_NAMES = ["a", "b", "c", "v", "x", "n", "f", "s", "q", "A", "V", "X", "verbose", "version", "verb", "help", "name", "number", "size", "define",
            "file", "files", "debug", "quiet", "Foo", "fo", "foo-bar", "n_x", "o", "level", "lib", "Include", "dry-run", "nof", "no-x"]
GL_TYPES = ["", "", "!", "!", "+", "=s", "=s", ":s", "=i", "=i", ":i", "=o", ":o", "=f", ":f", "=n", "=s@", ":s@", "=i@", "=s%", ":s%", "=i%",
            "=f@", "=s{2}", "=i{1,}", "=s{,2}", "=s@{0,2}", ":5", ":-3", ":+", ":+@", ":5@", ":007", "=o@", "=s{1,2}", ":i%"]
GL_BAD = ["foo=x", "g=s{0}", "f=s{0,0}", "h=i{3,1}", "<>", "k|=s", "|", "=s", "m!x", "p=s{2}", "1abc=s", "é=s", "q:+5", "r:5x", "s::s"]


def gl_specs(cfg):
    bundling = any(c.lower() in ("bundling", "bundling_override", "gnu_getopt") for c in cfg)
    used, specs, kinds = set(), [], []
    for _ in range(rng.choice([1, 2, 2, 3, 3, 4, 5, 6])):
        names = []
        for _ in range(rng.choice([1, 1, 1, 2, 3])):
            n = rng.choice(GL_NAMES)
            low = n.lower()
            if low in used or ("no" + low) in used or ("no-" + low) in used or (low.startswith("no") and low[2:].lstrip("-") in used):
                continue
            names.append(n)
            used.update([low, "no" + low, "no-" + low])
        if not names:
            continue
        t = rng.choice(GL_TYPES)
        while bundling and "{" in t and rng.random() < 0.9:
            t = rng.choice(GL_TYPES)
        spec = "|".join(names) + t
        kinds.append((names, t))
        if rng.random() < 0.08:
            spec = rng.choice(["--", "-", "+"]) + spec
        specs.append(spec)
    if rng.random() < 0.04:
        specs.insert(rng.randrange(len(specs) + 1), rng.choice(GL_BAD))
    if rng.random() < 0.08:
        specs.insert(0, rng.choice(["-", "+-", "/", "/", "-/"]))
    return specs, kinds


GL_VALUES = ["5", "-3", "+7", "1_000", "0x1f", "0X1F", "0b101", "017", "08", "0", "1.5", ".5", "5.", "1e5", "-2.5e-3", "1_0.5", "abc", "", "-x",
             "k=v", "k=5", "=v", "a=b=c", "99999999999999999999", "18446744073709551615", "-9223372036854775809", "0x10000000000000000",
             "0x1_f", "0x1__f", "1_0", "12\n", "0x1f\n", "-", "--", "+", "é", "x y", "0b1_0", "0o17", "007", "9223372036854775807",
             "0x100000000", "0b" + "1" * 40, "0" + "7" * 12, "-_5", "+_5", "1x5", "3:4", "1.5\n"]


def gl_value(t):
    if rng.random() < 0.25:
        return rng.choice(GL_VALUES)
    if "s" in t:
        v = rng.choice(["abc", "file.txt", "", "x y", "-", "é", "a=b", "--x", "-v"])
    elif "f" in t:
        v = rng.choice(["1.5", ".25", "-2", "1e3", "3", "1_0.5", "+0.5e-2"])
    elif "o" in t:
        v = rng.choice(["0x1f", "017", "0b101", "42", "-5", "0", "0x" + "f" * 9])
    else:
        v = rng.choice(["5", "-3", "+7", "0", "42", "1_000", "99999999999999999999", "18446744073709551615"])
    if "%" in t:
        v = rng.choice(["k", "key", "a b", "", "é"]) + rng.choice(["=", "=", ""]) + v
    return v


def gl_option_words(kinds, starters):
    names, t = rng.choice(kinds)
    n = rng.choice(names)
    if rng.random() < 0.15 and len(n) > 1:
        n = n[:rng.randrange(1, len(n))]
    if rng.random() < 0.08:
        n = n.upper()
    if "!" in t and rng.random() < 0.4:
        n = rng.choice(["no", "no-"]) + n
    starter = "-" if len(n) == 1 and rng.random() < 0.7 else rng.choice(["--", "--", "-", "+"])
    if starters:
        starter = rng.choice(starters)
    if t in ("", "!", "+") or rng.random() < 0.15:
        return [starter + n]
    if rng.random() < 0.06:
        return [starter + n + "="]
    v = gl_value(t)
    r = rng.random()
    if r < 0.35:
        return [starter + n + "=" + v]
    if r < 0.5 and len(n) == 1:
        return ["-" + n + v]
    words = [starter + n, v if rng.random() < 0.93 else "--"]
    if "{" in t or "@" in t:
        words += [gl_value(t) for _ in range(rng.choice([0, 1, 2]))]
    return words


def gl_word(kinds, starters):
    r = rng.random()
    if r < 0.12:
        single = [n for names, t in kinds for n in names if len(n) == 1] or ["v", "x"]
        return ["-" + "".join(rng.choice(single) for _ in range(rng.randrange(1, 4))) + rng.choice(["", "", "5", "1_0", "0x1f", "1.5", ".x", "k=3", "=4"])]
    if r < 0.25:
        return [rng.choice(GL_VALUES)]
    if r < 0.32:
        return [rng.choice(["--zzz", "-z", "file1", "--", "-", "+", "--=x", "-+", "---x", "--foo=", "+-x", "/v", "/x", "-/"])]
    return gl_option_words(kinds, starters) if kinds else ["file"]


def perlgetoptlong_line():
    h = lambda xs: ",".join(hx(x) for x in xs) if xs else "~"
    cfg = rng.choice(GL_CONFIGS) if rng.random() < 0.97 else rng.choice([["bogus"], ["no_default"], ["Bundling", "frob"]])
    specs, kinds = gl_specs(cfg)
    args = []
    starters = list(specs[0]) if specs and all(not c.isalnum() and c != "_" for c in specs[0]) else []
    for _ in range(rng.choice([0, 1, 2, 3, 3, 4, 5, 6])):
        args += gl_word(kinds, starters)
    return f"{h(cfg)} {h(specs)} {h(args)}"


family("perlgetoptlong", "perlgetoptlong", [perlgetoptlong_line() for _ in range(int(os.environ.get("PERLGETOPTLONG_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perlgetoptlong/oracle.pl")])


# --- Perl's Text::Balanced, against Perl 5.34 ------------------------------------------------------------

TB_PIECES = ["'", '"', "`", "(", ")", "[", "]", "{", "}", "<", ">", "\\", "\\", "a", "bc", " ", "  ", "\n", "\t", "x", "/", "|", "é", "☺", "\u00a0",
             "\u2028", "'a'", '"b"', "(c)", "[d]", "''", "\\'", '\\"', "#", ",", ";", "q", "1", "\u0085"]
TB_DELIMS = [None, None, "'", '"', "/", "'\"", "|", "ab", " ", "", "x", "\\", "''", "`", "'\"`", "/|", "é", "#"]
TB_ESCAPES = [None, None, "\\", "'", "/", "!", "", "0", "\\'", "x", "00", "|"]
TB_PREFIXES = [None, None, None, "", "\\s+", "[ \\t]*", " *", "#", "x*", ".", "a?", "\\s*#\\s*", "[,;]\\s*", "\\S*", "[^(]*", ".*", "\\s*\\s*",
               "[a-c]*", "[\\s,]*", "\\.?", "[]a]*", "\\(?", "[^\\s]+"]
TB_BRACKETS = [None, None, "()", "[]", "{}", "<>", "(", ")", "([", "'(", '"[', "`{", "(\"'", "x", "", "((", "]", "{(", "ab", "(☺", "(x(", ")]"]


def tb_text():
    return "".join(rng.choice(TB_PIECES) for _ in range(rng.randrange(0, 12)))


TB_LEADS = {None: ["", " ", "  ", "\t", "\n ", "\u00a0", "\u2028"], "": [""], "\\s+": [" ", "\t ", "\u2028"], "[ \\t]*": ["", " ", "\t"],
            " *": ["", " "], "#": ["#"], "x*": ["", "x", "xx"], ".": ["a", "#", " "], "a?": ["", "a"], "\\s*#\\s*": ["#", " # ", "#  "],
            "[,;]\\s*": [",", "; "], "\\S*": ["", "ab", "x"], "[^(]*": ["ab ", ""], ".*": ["", "ab"], "\\s*\\s*": [" ", ""],
            "[a-c]*": ["ab", ""], "[\\s,]*": [" , ", ""], "\\.?": [".", ""], "[]a]*": ["]a", ""], "\\(?": ["(", ""], "[^\\s]+": ["ab"]}


def tb_lead(pre):
    if rng.random() < 0.85:
        return rng.choice(TB_LEADS.get(pre, [""]))
    return rng.choice(["", " ", "#", "x", "ab", "\u00a0", ";"])


def tb_quoted(dels, escs):
    dels = dels if dels is not None else "'\"`"
    d = rng.choice(dels) if dels else "'"
    e = rng.choice(escs) if escs else "\\"
    body = "".join(rng.choice(["a", " ", e + d, e + e, e + "\n", d + d, "\n", "é", e, "b", d if rng.random() < 0.1 else "c"])
                   for _ in range(rng.randrange(0, 6)))
    return d + body + (d if rng.random() < 0.85 else "")


def tb_nested(opens, quotes, depth=0):
    if depth > 3:
        return ""
    o = rng.choice(opens)
    c = {"(": ")", "[": "]", "{": "}", "<": ">"}.get(o, o)
    q = rng.choice(quotes + "'") 
    pieces = ["a", " ", "\\" + rng.choice("()[]{}<>'\"x"), q + "x" + q, q + "(" + q, q + "\\" + q + q, q, "\n", "é", "b",
              rng.choice(")]}>") if rng.random() < 0.08 else "c"]
    inner = "".join(tb_nested(opens, quotes, depth + 1) if rng.random() < 0.3 else rng.choice(pieces) for _ in range(rng.randrange(0, 5)))
    return o + inner + (c if rng.random() < 0.9 else rng.choice(["", ")", "]"]))


def perltextbalanced_line():
    o = lambda v: "~" if v is None else hx(v)
    pre = rng.choice(TB_PREFIXES)
    if rng.random() < 0.5:
        dels, esc = rng.choice(TB_DELIMS), rng.choice(TB_ESCAPES)
        text = tb_lead(pre) + tb_quoted(dels, esc) + tb_text() if rng.random() < 0.8 else tb_text()
        return f"d {hx(text)} {o(dels)} {o(pre)} {o(esc)}"
    br = rng.choice(TB_BRACKETS)
    spec = br if br is not None else "{([<"
    opens = "".join(c for c in spec if c in "([{<") or "("
    if rng.random() < 0.15:
        opens = "([{<"
    quotes = "".join(c for c in spec if c in "'\"`")
    text = tb_lead(pre) + tb_nested(opens, quotes) + tb_text() if rng.random() < 0.8 else tb_text()
    return f"b {hx(text)} {o(br)} {o(pre)}"


family("perltextbalanced", "perltextbalanced", [perltextbalanced_line() for _ in range(int(os.environ.get("PERLTEXTBALANCED_LINES", "10000")))],
       ["perl", os.path.join(LIB, "perltextbalanced/oracle.pl")])


# --- SBCL's FORMAT for numbers, against SBCL 2.6.8 --------------------------------------------------------

def lf_single(x):
    return "s%08x" % struct.unpack("<I", struct.pack("<f", x))[0]


def lf_double(x):
    return "d%016x" % dbits(x)


LF_DOUBLES = [0.0, -0.0, 1.0, 0.5, 0.125, 2.5, 1.5, 0.05, 0.005, 3.14159, 123.456, 1e7, 1e-7, 1e100, 1e-100, 1.5e300, 5e-324, 1e-320,
              2.2250738585072014e-308, 1.7976931348623157e308, 0.1, 1 / 3, 2 / 3, 9.999, 99.95, 0.0001, 123456789.0, 1234567.0, 0.001,
              1e21, 1e-5, 999999.5, 0.95, 10.0, 100.0, 1e15, 1e16, 12345.678]


def lf_float():
    r = rng.random()
    if r < 0.35:
        return rng.choice(LF_DOUBLES) * rng.choice([1, 1, -1])
    if r < 0.55:
        return rng.randrange(-10 ** 6, 10 ** 6) / rng.choice([1, 2, 4, 8, 10, 100, 1000, 3])
    if r < 0.8:
        return rng.uniform(-1, 1) * 10.0 ** rng.randrange(-12, 15)
    return struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]


def lf_arg(kind):
    r = rng.random()
    if kind == "int":
        if r < 0.5:
            return "i" + str(rng.choice([0, 1, -1, 7, 42, 255, -255, 1000, 1234567, -1234567, 2 ** 31, 2 ** 62, -2 ** 63, 10 ** 20, -10 ** 25, 2 ** 100]))
        if r < 0.85:
            return "i" + str(rng.randrange(-10 ** rng.randrange(1, 12), 10 ** rng.randrange(1, 12)))
        return rng.choice(["r1/3", "r-22/7", "r255/16", lf_double(1.5), lf_single(1e7), "n", "c97", lf_double(float("inf")), lf_double(float("nan")),
                           lf_single(-0.0), lf_double(1e-7), lf_double(0.001), lf_double(1e16), lf_double(1234567.0), lf_single(9999999.0),
                           lf_double(lf_float()), lf_single(rng.uniform(-1, 1) * 10.0 ** rng.randrange(-8, 10))])
    if r < 0.5:
        x = lf_float()
        return lf_double(x) if x != x or abs(x) > 3.4e38 or rng.random() < 0.6 else lf_single(x)
    if r < 0.65:
        return lf_single(rng.choice([1.0, 0.5, 0.125, 2.5, 1e-45, 1e38, 3.4028235e38, 1.1754944e-38, 0.1, 1e7, 16777216.0, 0.3]) * rng.choice([1, -1]))
    if r < 0.8:
        return "i" + str(rng.choice([0, 1, -3, 5, 100, 16777217, 2 ** 62 + 2 ** 38 + 1, 2 ** 62 + 2 ** 38, 10 ** 20, 2 ** 128, 2 ** 128 - 2 ** 103 - 1,
                                     rng.randrange(-10 ** 9, 10 ** 9)]))
    if r < 0.92:
        return "r" + rng.choice(["1/3", "-2/3", "22/7", "1/1000", "1/" + str(10 ** 45), "1/" + str(10 ** 46), "3/" + str(2 ** 150), "1/8", "5/2",
                                 str(rng.randrange(-10 ** 6, 10 ** 6)) + "/" + str(rng.randrange(1, 10 ** 4))])
    return rng.choice([lf_double(float("inf")), lf_double(float("-inf")), lf_double(float("nan")), "d7ff4000000000000", lf_single(float("inf")),
                       "s7fc00000", "n", "c120"])


def lf_param(kind, args):
    r = rng.random()
    if r < 0.45:
        return ""
    if kind == "char":
        if r < 0.85:
            return "'" + rng.choice(["*", "0", " ", "x", "#", "é", ",", "."])
        args.append("c" + str(ord(rng.choice("*0x"))))
        return "v"
    if r < 0.85:
        return str(rng.choice([0, 1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 20, -1, -2, 7, 9]))
    if r < 0.93:
        args.append(rng.choice(["i" + str(rng.randrange(-2, 12)), "n"]))
        return "v"
    return "#"


LF_SHAPES = {"F": ["int", "int", "int", "char", "char"], "E": ["int", "int", "int", "int", "char", "char", "char"],
             "G": ["int", "int", "int", "int", "char", "char", "char"], "$": ["int", "int", "int", "char"],
             "D": ["int", "char", "char", "int"], "B": ["int", "char", "char", "int"], "O": ["int", "char", "char", "int"],
             "X": ["int", "char", "char", "int"]}


def lf_directive(args):
    c = rng.choice(["F", "F", "F", "E", "E", "G", "G", "$", "D", "D", "B", "O", "X"])
    shape = LF_SHAPES[c]
    n = rng.choice([0, 0, 1, 2, 3, len(shape), rng.randrange(len(shape) + 1)])
    params = [lf_param(shape[i] if i < len(shape) else "int", args) for i in range(n)]
    if c in "DBOX" and len(params) > 3 and params[3] not in ("", "v", "#"):
        params[3] = str(rng.randrange(1, 5))
    if c in "DBOX" and len(params) > 3 and params[3] in ("v", "#"):
        if params[3] == "v":
            args.pop()
        params[3] = "2"
    if c in "DBOX" and params and params[0] not in ("", "v", "#") and int(params[0]) < 0:
        params[0] = "3"
    mods = rng.choice(["", "", "", "@", ":", ":@", "@:"]) if c in "$DBOX" else rng.choice(["", "", "@"])
    if rng.random() < 0.02:
        mods = rng.choice([":", "::", "@@"])
    if rng.random() < 0.04 and len(params) == len(shape):
        params.append("")
    args.append(lf_arg("int" if c in "DBOX" else "float"))
    return "~" + ",".join(params) + mods + (c.lower() if rng.random() < 0.1 else c)


LF_EDGES = [9.9999, 99.999, 999.95, 0.99995, 9.995, 99.5, 9.5, 0.95, 0.0995, 999999.5, 9.96, 99.96, 1.995, 19.95, 0.9996, 9999.9]


def lispformat_line():
    if rng.random() < 0.1:
        x = rng.choice(LF_EDGES) * rng.choice([1, 1, -1]) * rng.choice([1, 1, 10, 0.1, 100])
        arg = lf_double(x) if rng.random() < 0.6 else lf_single(x)
        return " ".join([hx("~" + str(rng.randrange(1, 9)) + rng.choice(["F", "F", "E", "G", ",,1E", ",,,'*F"])), arg])
    pieces, args = [], []
    for _ in range(rng.choice([1, 1, 1, 2, 3])):
        pieces.append(rng.choice(["", "", "|", " ", "x=", "é", "~%", "~~", "~2%", "<"]))
        pieces.append(lf_directive(args))
    pieces.append(rng.choice(["", "", "|", " "]) if rng.random() < 0.97 else "~")
    if rng.random() < 0.03 and args:
        args.pop()
    return " ".join([hx("".join(pieces))] + args)


family("lispformat", "lispformat", [lispformat_line() for _ in range(int(os.environ.get("LISPFORMAT_LINES", "10000")))],
       ["sbcl", "--script", os.path.join(LIB, "lispformat/oracle.lisp")])


# --- SBCL's reader on number tokens, against SBCL 2.6.8 ---------------------------------------------------

LR_PIECES = ["0", "1", "2", "5", "7", "9", "12", "100", "255", "007", ".", ".", "e", "E", "d", "D", "f", "s", "l", "L", "r", "+", "-", "/", "a", "b",
             "c", "z", "x", "A", "F", "ff", "e5", "e-3", "d0", "e+", "1/2", "\\", "|", "|a|", "\\a", "#", "_", "*", "<", "1.5", ".5", "5."]
LR_FORMS = ["1.", "17.", "-17.", "+17.", "1.5", ".5", "-.5", "+.5", "1.e5", "1e5", "1E5", "1d0", "1.5d0", "1.5s0", "1.5f0", "1.5l0", "1.5r0", "1r2",
            "1/2", "-1/2", "4/2", "1/0", "0/5", "1/", "+.", "-.", "+", "-", ".", "..", "...", "1e", "1e+", "1e5x", "1.5.5", "1..", "abc", "|1|",
            "\\1", "1\\2", "1e400", "1d400", "1e-400", "1d-400", "1d-320", "1e-45", "3.4028235e38", "3.4028236e38", "1.7976931348623157d308",
            "1.7976931348623158d308", "4.9406564584124654d-324", "2.4703282292062328d-324", "2.4703282292062327d-324", "0.1", "-0.0", "-0", "+0.",
            "123456789012345678901234567890", "1.000000059604645", "1.0000000596046448", "0.30000000000000004d0", "ff", "1e+5", "a.", "ff.5",
            "-1e5", "d.5", "1s0", "e", "#1", "1#", "1_000", "\b", "1\x7f", "1 2", "12)", "12(", "12'x", "1.5;c", "  42", "\t7", "1.5e-3", "1.5E+3",
            "6.02e23", "6.02d23", "1e10", "1d10", "1/3", "-22/7", "10/4", "z/1", "16777217.0", "16777219.0", "-16777217.0",
            "9007199254740993.0d0", "9007199254740995.0d0", "1.000000059604644775390625", "2.00000011920928955078125", "0.5000000298023223876953125",
            "1.0000000000000001110223024625156540423631668090820312500d0", "1.00000005960464477539062501", "16777217.000001", "33554434.0",
            "1.401298464324817070923729583289916131280e-45", "7.006492321624085354618647916449580656401e-46", "2.4703282292062327208828439643411068618252990130716238221279284125033775364d-324"]
LR_BASES = [10, 10, 10, 10, 16, 16, 2, 8, 36, 3, 14, 15, 20, 22, 29, 30]
LR_FORMATS = ["single-float", "single-float", "double-float", "double-float", "short-float", "long-float"]


def lr_token():
    r = rng.random()
    if r < 0.4:
        return rng.choice(LR_FORMS)
    if r < 0.55:
        sign = rng.choice(["", "", "-", "+"])
        digits = "".join(rng.choice("0123456789") for _ in range(rng.randrange(1, 25)))
        frac = "".join(rng.choice("0123456789") for _ in range(rng.randrange(0, 25)))
        exp = rng.choice(["", "", "e" + str(rng.randrange(-60, 60)), "d" + str(rng.randrange(-330, 320)), "f" + str(rng.randrange(-50, 40)),
                          "E" + str(rng.randrange(-5, 5)), "e" + str(rng.randrange(-400, 400))])
        return sign + digits + ("." + frac if frac or rng.random() < 0.3 else "") + exp
    if r < 0.65:
        return rng.choice(["", "-", "+"]) + "".join(rng.choice("0123456789abcdefABCDEFxz") for _ in range(rng.randrange(1, 8))) + rng.choice(["", ".", "/3", "/0", "e5", "e+5"])
    return "".join(rng.choice(LR_PIECES) for _ in range(rng.randrange(1, 6)))


def lispreadnumber_line():
    t = lr_token()
    while t.strip(" \t") == "" and rng.random() < 0.9 or t.lstrip(" \t")[:1] in ("#", "(", ")", "'", '"', ";", "`", ","):
        t = lr_token()
    return f"{rng.choice(LR_BASES) if rng.random() < 0.9 else rng.randrange(2, 37)} {rng.choice(LR_FORMATS)} {hx(t)}"


family("lispreadnumber", "lispreadnumber", [lispreadnumber_line() for _ in range(int(os.environ.get("LISPREADNUMBER_LINES", "10000")))],
       ["sbcl", "--script", os.path.join(LIB, "lispreadnumber/oracle.lisp")])


# --- Rust's format specs, against rustc 1.99 ---------------------------------------------------------------

RF_FILLS = [" ", "*", "0", "_", "é", "-", "<", ">", "^", "+", "#", ".", "9", "x", "☺"]
RF_INTS = [0, 1, -1, 7, 42, 255, -255, 1000, 1234, 1250, 1235, 25, 35, 45, 99999, 100, -100, 2 ** 31, -2 ** 31, 2 ** 63 - 1, -2 ** 63, 10 ** 18,
           -10 ** 18, 999999999999, 123456789, 10 ** 15 + 5]
RF_U64S = [0, 1, 2 ** 64 - 1, 2 ** 63, 2 ** 63 - 1, 10 ** 19, 18446744073709551615 - 5, 255, 1000]
RF_FLOATS = [0.0, -0.0, 1.0, -1.5, 0.1, 0.125, 2.5, 1e16, 9999999999999998.0, 1e-4, 0.00009999, 1e-5, 1e21, 123.456, 1e300, 5e-324,
             float("inf"), float("-inf"), float("nan"), 3.14159, 0.5, 1.005, 2.675, 1e15, 0.3]
RF_STRS = ["", "ab", "héllo", "日本語", "a\u0301b", "😀x", "abcdefghij", " sp ", "x"]


def rf_value():
    r = rng.random()
    if r < 0.35:
        return "int", "i" + str(rng.choice(RF_INTS) if rng.random() < 0.6 else rng.randrange(-2 ** 63, 2 ** 63) // rng.choice([1, 1, 10 ** 6, 10 ** 12]))
    if r < 0.45:
        return "int", "u" + str(rng.choice(RF_U64S) if rng.random() < 0.6 else rng.randrange(2 ** 64))
    if r < 0.8:
        x = rng.choice(RF_FLOATS) if rng.random() < 0.5 else (rng.uniform(-1, 1) * 10.0 ** rng.randrange(-8, 20) if rng.random() < 0.7 else
                                                             struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0])
        return "float", "f%016x" % dbits(x)
    return "str", "s" + hx(rng.choice(RF_STRS))


def rustfmt_line():
    if rng.random() < 0.05:
        n = rng.choice([25, 35, 45, 15, 125, 135, 1250, 1350, 12345, 2500, 3500, 105, 995, 9950, 99950, 1000005, 2 ** 63 - 1, 18446744073709551615])
        sign = rng.choice(["", "-"]) if n < 2 ** 63 else ""
        return " ".join([hx(rng.choice(["", "+", ">12"]) + "." + str(rng.randrange(0, 4)) + rng.choice("eE")), ("u" if n >= 2 ** 63 else "i" + sign) + str(n)])
    kind, value = rf_value()
    spec, counts = "", []
    if rng.random() < 0.45:
        if rng.random() < 0.4:
            spec += rng.choice(RF_FILLS)
        spec += rng.choice("<^>")
    if rng.random() < 0.2:
        spec += rng.choice(["+", "+", "-"])
    if rng.random() < 0.25:
        spec += "#"
    if rng.random() < 0.2:
        spec += "0"
    r = rng.random()
    star = False
    if r < 0.45:
        spec += str(rng.randrange(0, 26))
    elif r < 0.55:
        counts.append(str(rng.randrange(0, 30)))
        spec += str(len(counts)) + "$"
    r = rng.random()
    if r < 0.3:
        spec += "." + str(rng.randrange(0, 22))
    elif r < 0.36:
        counts.append(str(rng.randrange(0, 20)))
        spec += "." + str(len(counts)) + "$"
    elif r < 0.4 and not counts:
        counts.append(str(rng.randrange(0, 20)))
        spec += ".*"
        star = True
    if kind == "int":
        spec += rng.choice(["", "", "?", "x?", "X?", "x", "X", "o", "b", "e", "E", "#?"[1:]])
    elif kind == "float":
        spec += rng.choice(["", "", "?", "e", "E"])
    if star and len(counts) != 1:
        return rustfmt_line()
    return " ".join([hx(spec), value] + counts)


family("rustfmt", "rustfmt", [rustfmt_line() for _ in range(int(os.environ.get("RUSTFMT_LINES", "10000")))],
       ["python3", os.path.join(LIB, "rustfmt/oracle.py")])


# --- Lua's arithmetic, against Lua 5.5 ----------------------------------------------------------------------

LN_INTS = [0, 1, -1, 2, -2, 3, -3, 7, -7, 10, 63, 64, 65, -63, -64, -65, 100, 2 ** 53, 2 ** 53 + 1, -2 ** 53 - 1, 2 ** 62, 2 ** 63 - 1, -2 ** 63,
           -2 ** 63 + 1, 255, 1000000007, 12345678901234]
LN_FLOATS = [0.0, -0.0, 0.5, -0.5, 1.0, -1.0, 1.5, -1.5, 2.0, 2.5, 3.0, -7.5, 1e15, 2.0 ** 53, 2.0 ** 53 + 2, 2.0 ** 63, -2.0 ** 63, 2.0 ** 64,
             1e300, -1e300, float("inf"), float("-inf"), float("nan"), 0.1, 1 / 3, 5e-324, 64.0, -64.0, 1e-10, 9007199254740993.0, 3.7, -3.7]
LN_STRS = ["10", "-10", "3.0", " 0x10 ", "1e2", "0x7fffffffffffffff", "9223372036854775808", "-9223372036854775809", "abc", "", "1e", " 5 ",
           "0x1p4", "2", "2.0", "inf", "nan", "0x", "1 2", "  -0  ", "0.1", "1e400", "a", "b", "10\x00", "é", "Z", "-0x8000000000000000"]


def ln_value():
    r = rng.random()
    if r < 0.4:
        return "i" + str(rng.choice(LN_INTS) if rng.random() < 0.6 else rng.randrange(-10 ** rng.randrange(1, 19), 10 ** rng.randrange(1, 19)))
    if r < 0.75:
        x = rng.choice(LN_FLOATS) if rng.random() < 0.6 else rng.choice([rng.uniform(-100, 100), float(rng.randrange(-1000, 1000)),
                                                                        rng.uniform(-1, 1) * 10.0 ** rng.randrange(-5, 25)])
        return "f%016x" % dbits(x)
    return "s" + hx(rng.choice(LN_STRS))


LN_BINARY = ["+", "-", "*", "/", "//", "%", "//", "%", "&", "|", "~", "<<", ">>", "<", "<=", ">", ">=", "==", "~="]


LN_NEAR = [2 ** 53, 2 ** 53 + 1, 2 ** 53 + 2, 2 ** 53 + 3, 2 ** 53 + 4, -2 ** 53 - 1, -2 ** 53 - 3, 2 ** 54 + 1, 2 ** 62 + 1, 2 ** 63 - 1, -2 ** 63,
           2 ** 63 - 512, 2 ** 63 - 513]


def luanumber_line():
    if rng.random() < 0.06:
        i = rng.choice(LN_NEAR) * rng.choice([1, 1, -1]) if rng.random() < 0.9 else rng.choice(LN_NEAR)
        i = max(min(i, 2 ** 63 - 1), -2 ** 63)
        f = float(rng.choice(LN_NEAR)) * rng.choice([1, -1]) + rng.choice([0.0, 0.0, 0.5, -0.5]) if rng.random() < 0.6 else float(i)
        pair = [f"i{i}", "f%016x" % dbits(f)]
        rng.shuffle(pair)
        return f"{rng.choice(['<', '<=', '>', '>=', '==', '~=', 'm:max', 'm:min'])} {pair[0]} {pair[1]}"
    if rng.random() < 0.04:
        a = rng.choice(["i1", "f3ff0000000000000", "s" + hx("1"), "s" + hx("ab"), "s" + hx("abc"), "s" + hx("a"), "s" + hx(""), "i2"])
        b = rng.choice(["i1", "f3ff0000000000000", "s" + hx("ab"), "s" + hx("abc"), "s" + hx("b"), "s" + hx("a\x00b"), "s" + hx("a\x00")])
        return f"{rng.choice(['m:max', 'm:min', '<', '<=', '=='])} {a} {b}"
    r = rng.random()
    if r < 0.6:
        op = rng.choice(LN_BINARY)
        second = ln_value()
        if op in ("//", "%") and rng.random() < 0.15:
            second = rng.choice(["i0", "i0", "i-1", "f%016x" % dbits(0.0), "f%016x" % dbits(-0.0), "s" + hx("0")])
        return f"{op} {ln_value()} {second}"
    if r < 0.65:
        return f"^ {ln_value()} {rng.choice(['i2', 'f4000000000000000', 's' + hx('2'), 's' + hx('2.0')])}"
    if r < 0.72:
        return f"{rng.choice(['neg', 'bnot'])} {ln_value()}"
    if r < 0.9:
        name = rng.choice(["tointeger", "type", "floor", "ceil", "abs", "fmod", "max", "min", "ult", "fmod", "max", "min"])
        n = 2 if name in ("fmod", "ult") else (rng.choice([1, 2, 3]) if name in ("max", "min") else 1)
        return f"m:{name} " + " ".join(ln_value() for _ in range(n))
    digits = "".join(rng.choice("0123456789abcdefzZAF") for _ in range(rng.randrange(0, 20)))
    text = rng.choice(["", " ", "-", "+", "\t"]) + digits + rng.choice(["", " ", "\n", "x", " 1"])
    return f"tonumber s{hx(text)} i{rng.choice([2, 8, 10, 16, 36, 7, 1, 37, 0, 35])}"


family("luanumber", "luanumber", [luanumber_line() for _ in range(int(os.environ.get("LUANUMBER_LINES", "10000")))],
       ["lua", os.path.join(LIB, "luanumber/oracle.lua")])


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
            if shown < int(os.environ.get("SHOW", "6")):
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
