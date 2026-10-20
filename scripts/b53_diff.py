"""B53's packages against the references they reproduce.

- CPython 3.13's glob.translate (recursive, include_hidden, seps), escape
  and has_magic.
- Go 1.25's encoding/base64 (standard, URL, raw, other padding, strict)
  and encoding/base32 (standard, hex, raw, other padding).
- Go 1.25's encoding/hex: EncodeToString, DecodeString and Dump.
- Go 1.25's encoding/ascii85: Encode, and Decode with and without flush.
- Java 17's java.math.BigInteger: parsing, printing, byte arrays, bit
  operations, shifts, division, mod, modPow, modInverse, gcd, pow, sqrt
  and the long and int conversions.
- CPython 3.13's binascii.b2a_qp and a2b_qp, the code under quopri.
- CPython 3.13's email.utils: parseaddr, getaddresses, formataddr, quote
  and unquote.
- Ruby 2.6's String#succ, tr, tr_s, delete, squeeze and count on byte
  strings.
- Node 25's StringDecoder: write and end over chunked input, in every
  encoding it accepts.
- CPython 3.13's http.cookies: SimpleCookie.load, setting values and
  attributes, and output, CookieError messages included.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B53_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B53_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B53_LIB") or os.path.join(ROOT, "scripts/lib/b53")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b53")
harness = os.path.join(work, "b53test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b53test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


# --- Python's glob, against CPython 3.13 -----------------------------------------------------------------------

GL_PIECES = ["a", "b", "*", "**", "?", ".", "..", ".x", "*.py", "?.c", "[ab]", "[!.]", "[a-c-e]", "[", "é", "\\", "a*b", "**x", "x**", "[]a]", "[!]", "*?",
             "~", "+", "(", "|"]


def gl_pattern(seps):
    if rng.random() < 0.15:
        sep = rng.choice(seps or "/")
        return rng.choice(["**", "**/**", "a/**/**/b", "**/*.py", "*/**", "**/", ".**", "**/**/**", "x/**/**"]).replace("/", sep)
    out = []
    for _ in range(rng.randrange(0, 5)):
        out.append("".join(rng.choice(GL_PIECES) for _ in range(rng.randrange(0, 3))))
    return rng.choice(seps or "/").join(out) + rng.choice(["", "", "/"])


def pyglob_line():
    r = rng.random()
    if r < 0.75:
        seps = rng.choice(["", "", "", "/", "\\", "/\\", ":", "./"])
        return f"tr {gl_pattern(seps).encode().hex() or '-'} {rng.choice('01')} {rng.choice('01')} {seps.encode().hex() or '-'}"
    t = gl_pattern("")
    return f"{rng.choice(['es', 'hm'])} {t.encode().hex() or '-'}"


family("pyglob", "pyglob", [pyglob_line() for _ in range(int(os.environ.get("PYGLOB_LINES", "6000")))], ["python3.13", os.path.join(LIB, "pyglob/oracle.py")])


# --- Go's base64 and base32, against Go 1.25 ---------------------------------------------------------------------

B64_ENCODINGS = ["std", "url", "rawstd", "rawurl", "stdstar", "stdS", "urlS", "rawstdS", "rawurlS"]
B32_ENCODINGS = ["std", "hex", "rawstd", "rawhex", "stdstar"]


def b64_text(alphabet, pad):
    import base64
    r = rng.random()
    data = bytes(rng.randrange(256) for _ in range(rng.randrange(0, 12)))
    if alphabet == "64":
        t = base64.b64encode(data).decode()
    else:
        t = base64.b32encode(data).decode()
    if pad != "=":
        t = t.rstrip("=") + (pad * (len(t) - len(t.rstrip("="))) if pad else "")
    if r < 0.5:
        return t
    cs = list(t)
    for _ in range(rng.randrange(1, 3)):
        i = rng.randrange(0, len(cs) + 1)
        k = rng.random()
        if k < 0.3 and cs:
            del cs[min(i, len(cs) - 1)]
        elif k < 0.6:
            cs.insert(i, rng.choice(["\n", "\r", "=", "*", "-", "_", "+", "/", "A", "0", " ", "!", "Q", "B"]))
        elif cs:
            cs[min(i, len(cs) - 1)] = rng.choice(["=", "A", "B", "Q", "7", "z", "\n"])
    return "".join(cs)


def gobase64_line():
    r = rng.random()
    if r < 0.15:
        return f"e64 {rng.choice(B64_ENCODINGS)} {bytes(rng.randrange(256) for _ in range(rng.randrange(0, 10))).hex() or '-'}"
    if r < 0.6:
        e = rng.choice(B64_ENCODINGS)
        pad = "" if "raw" in e else "*" if "star" in e else "="
        t = b64_text("64", pad)
        if e.startswith("url"):
            t = t.replace("+", "-").replace("/", "_")
        return f"d64 {e} {t.encode().hex() or '-'}"
    if r < 0.7:
        return f"e32 {rng.choice(B32_ENCODINGS)} {bytes(rng.randrange(256) for _ in range(rng.randrange(0, 12))).hex() or '-'}"
    e = rng.choice(B32_ENCODINGS)
    pad = "" if "raw" in e else "*" if "star" in e else "="
    t = b64_text("32", pad)
    if e.startswith("hex"):
        t = t.translate(str.maketrans("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567", "0123456789ABCDEFGHIJKLMNOPQRSTUV"))
    return f"d32 {e} {t.encode().hex() or '-'}"


family("goencoding-base64", "gobase64", [gobase64_line() for _ in range(int(os.environ.get("GOBASE64_LINES", "8000")))],
       [go_tool("gobase64", os.path.join(LIB, "goencoding/base64/oracle.go"))])


# --- Go's encoding/hex, against Go 1.25 -------------------------------------------------------------------------

def gohexdump_line():
    r = rng.random()
    data = bytes(rng.randrange(256) for _ in range(rng.choice([0, 1, 7, 8, 9, 15, 16, 17, 31, 32, 33, rng.randrange(0, 70)])))
    if r < 0.2:
        return f"en {data.hex() or '-'}"
    if r < 0.6:
        t = data.hex()
        if rng.random() < 0.3:
            t = t.upper()
        cs = list(t)
        if rng.random() < 0.6:
            for _ in range(rng.randrange(1, 3)):
                i = rng.randrange(0, len(cs) + 1)
                cs.insert(i, chr(rng.choice([0x67, 0x20, 0x00, 0x7f, 0xe9, 0xa0, 0xad, 0x2d, 0x46, 0x30])))
        raw = "".join(cs).encode("latin-1")
        return f"de {raw.hex() or '-'}"
    return f"du {data.hex() or '-'}"


family("goencoding-hex", "gohexdump", [gohexdump_line() for _ in range(int(os.environ.get("GOHEXDUMP_LINES", "6000")))],
       [go_tool("gohexdump", os.path.join(LIB, "goencoding/hex/oracle.go"))])


# --- Go's encoding/ascii85, against Go 1.25 --------------------------------------------------------------------

def a85_encode(data):
    import base64
    return base64.a85encode(data).decode()


def goascii85_line():
    data = rng.choice([bytes(rng.randrange(256) for _ in range(rng.randrange(0, 14))), b"\x00" * rng.randrange(0, 9), b"\xff" * rng.randrange(0, 9)])
    if rng.random() < 0.25:
        return f"en {data.hex() or '-'}"
    t = a85_encode(data)
    cs = list(t)
    if rng.random() < 0.6:
        for _ in range(rng.randrange(1, 3)):
            i = rng.randrange(0, len(cs) + 1)
            cs.insert(i, rng.choice(["z", "u", " ", "\n", "\x00", "~", "v", "!", "{", "zz", "uuuuu"]))
    return f"de {''.join(cs).encode().hex() or '-'} {rng.choice('01')}"


family("goencoding-ascii85", "goascii85", [goascii85_line() for _ in range(int(os.environ.get("GOASCII85_LINES", "6000")))],
       [go_tool("goascii85", os.path.join(LIB, "goencoding/ascii85/oracle.go"))])


# --- Java's BigInteger, against Java 17 -------------------------------------------------------------------------

def jb_int():
    r = rng.random()
    if r < 0.3:
        return rng.choice([0, 1, -1, 2, -2, 7, 255, 256, -256, -255, 2 ** 31 - 1, -2 ** 31, 2 ** 63 - 1, -2 ** 63, 2 ** 63, -2 ** 63 - 1, 2 ** 64, 10 ** 20])
    return rng.randrange(-10 ** rng.randrange(1, 40), 10 ** rng.randrange(1, 40))


def jb_text(radix):
    digits = "0123456789abcdefghijklmnopqrstuvwxyz"[:radix]
    r = rng.random()
    if r < 0.15:
        return rng.choice(["", "-", "+", "+-1", "1-", "--1", "0", "-0", "000", "١٢٣", "１２", "ＦＦ", "ｆｆ", "12x4", " 1", "1 ", "٣", "+0", "७", "๙9", "෧"])
    body = "".join(rng.choice(digits + digits.upper()) for _ in range(rng.randrange(1, 40)))
    if rng.random() < 0.15:
        i = rng.randrange(len(body) + 1)
        body = body[:i] + rng.choice(["x", "g", "!", "-", "z", "é", " ", "𝟗", "𝟗𝟗", "٣", "𞥐"]) + body[i:]
    return rng.choice(["", "", "-", "+"]) + rng.choice(["", "0", "000"]) + body


def javabiginteger_line():
    r = rng.random()
    if r < 0.2:
        radix = rng.choice([10, 10, 16, 2, 8, 36, 7, 1, 37])
        return f"pa {jb_text(radix if 2 <= radix <= 36 else 10).encode().hex() or '-'} {radix}"
    if r < 0.27:
        return f"ts {jb_int()} {rng.choice([10, 16, 2, 36, 7, 1, 37])}"
    if r < 0.33:
        return f"ba {jb_int()}"
    if r < 0.38:
        return f"fb {bytes(rng.randrange(256) for _ in range(rng.randrange(0, 12))).hex() or '-'}"
    if r < 0.43:
        return f"bl {jb_int()}"
    if r < 0.55:
        return f"bw {rng.choice(['and', 'or', 'xor', 'andnot', 'not'])} {jb_int()} {jb_int()}"
    if r < 0.62:
        return f"sh {jb_int()} {rng.choice([0, 1, 3, 8, 64, -1, -3, -64, 100])}"
    if r < 0.7:
        return f"tb {jb_int()} {rng.choice([0, 1, 7, 63, 64, 100, -1])}"
    if r < 0.88:
        k = rng.choice(["div", "rem", "mod", "gcd", "inv", "pow", "sqrt"])
        b = rng.choice([0, 1, 2, 3, -5, 7, 30, -1]) if k == "pow" else (jb_int() if rng.random() < 0.85 else 0)
        return f"ar {k} {jb_int()} {b}"
    if r < 0.95:
        return f"mp {jb_int()} {rng.choice([0, 1, 2, -1, -3, 65537, jb_int()])} {rng.choice([1, 2, 3, 7, 8, 1000, 0, -5, 2 ** 64 + 13, 10 ** 12 + 39])}"
    return f"lv {jb_int()}"


family("javabiginteger", "javabiginteger", [javabiginteger_line() for _ in range(int(os.environ.get("JAVABIGINTEGER_LINES", "6000")))],
       ["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "BigIntegerOracle"])


# --- Python's quoted-printable, against CPython 3.13 -----------------------------------------------------------

QP_PIECES = [b"a", b"hello", b" ", b"\t", b"  ", b"\n", b"\r\n", b"\r", b"=", b"_", b".", b"\x00", b"\xe9", b"\xff", b"\x7f", b"x" * 70, b"y" * 74,
             b"z" * 75, b"w" * 73, b"=41", b"=4", b"=g1", b"==", b"=\n", b"=\r\n", b"=\r", b"= \n", b"=ab", b"=AB", b"\x0c"]


def pyquopri_line():
    data = b"".join(rng.choice(QP_PIECES) for _ in range(rng.randrange(0, 10)))
    if rng.random() < 0.55:
        return f"en {data.hex() or '-'} {rng.choice('01')} {rng.choice('011')} {rng.choice('01')}"
    if rng.random() < 0.5:
        import binascii
        data = binascii.b2a_qp(data, quotetabs=rng.random() < 0.3, header=rng.random() < 0.3)
    return f"de {data.hex() or '-'} {rng.choice('01')}"


family("pyquopri", "pyquopri", [pyquopri_line() for _ in range(int(os.environ.get("PYQUOPRI_LINES", "6000")))],
       ["python3.13", os.path.join(LIB, "pyquopri/oracle.py")])


# --- Python's email.utils, against CPython 3.13 ------------------------------------------------------------------

EM_ADDR_PIECES = ["a@b.c", "x.y@z.org", "user@[1.2.3.4]", "\"q x\"@d.e", "\"a\\\"b\"@c", "a . b @ c . d", "a@b", "@b", "a@", "a", "", "<a@b.c>", "<@r.s:a@b.c>",
                  "<@r,@s:a@b>", "<>", "<a@b", "a@b>", "jü@x", "a@b.c.", ".a@b", "a..b@c", "a@[1.2", "[x]@y", "a@b (Comment)", "(c) a@b", "a(c)@b"]
EM_NAMES = ["John Doe", "\"Doe, John\"", "\"a\\\"b\"", "Jürgen", "J. Smith", "\"\"", "(only comment)", "Name (with comment)", "\"x(y\"", "a\\,b", "=?utf-8?q?x?=",
            "Mr  Spaced"]


def em_address():
    r = rng.random()
    if r < 0.4:
        return rng.choice(EM_ADDR_PIECES)
    if r < 0.75:
        return rng.choice(EM_NAMES) + rng.choice([" ", "", "  "]) + rng.choice(["<a@b.c>", "<x.y@z>", "<@r:a@b>", "<a@[1.2.3.4]>", "<>", "<a@b", "<\"q\"@d>"]) + \
            rng.choice(["", "", " (trailing)", " (t (nested))"])
    if r < 0.85:
        return "group" + rng.choice([":", ": "]) + rng.choice(["a@b, c@d", "", "x <y@z>", "a@b"]) + rng.choice([";", "", "; "])
    return "".join(rng.choice(["a", "@", "b", ".", "<", ">", "(", ")", "\"", ",", ":", ";", "\\", " ", "[", "]", "\r\n ", "é"]) for _ in range(rng.randrange(1, 12)))


def em_field():
    return rng.choice([", ", ",", " , ", "\r\n\t,"]).join(em_address() for _ in range(rng.randrange(1, 4)))


def em_hex(t):
    return t.encode().hex() or "-"


def pyemailutils_line():
    r = rng.random()
    if r < 0.4:
        return f"pa {rng.choice('011')} {em_hex(em_field() if rng.random() < 0.3 else em_address())}"
    if r < 0.7:
        return "ga " + rng.choice("011") + " " + " ".join(em_hex(em_field()) for _ in range(rng.randrange(1, 4)))
    if r < 0.9:
        name = rng.choice(["", "John Doe", "Doe, John", "a\"b", "back\\slash", "Jürgen Müller", "日本語", "x y z", "dot.name", "émile", "a b", "€", "😀",
                           "ÿÿÿÿ", "ascii_ok-1", "a=b", "Ünïcödé, Name", "Jean-Pierre Lefèvre", "Ole Gunnar Solskjær", "abcdefg€", "Ann (é) Lee",
                           "Mr. Lefèvre-Durand"])
        addr = rng.choice(["a@b.c", "x@y", "", "a@b.c", "x@y", "jé@x", "jé€@x", "€€@x", "\x80@x"])
        return f"fa {em_hex(name)} {em_hex(addr)}"
    t = rng.choice(["\"a\\\\b\\\"c\"", "<x@y>", "\"", "<", "\"\"", "<>", "a\\b\"c", "\"unterminated", "plain", "\"\\\\\\\\\""])
    return f"{rng.choice(['qu', 'uq'])} {em_hex(t)}"


family("pyemailutils", "pyemailutils", [pyemailutils_line() for _ in range(int(os.environ.get("PYEMAILUTILS_LINES", "8000")))],
       ["python3.13", os.path.join(LIB, "pyemailutils/oracle.py")])


# --- Ruby's String#succ, tr, delete, squeeze and count, against Ruby 2.6 ---------------------------------------

RS_BYTES = b"aazZ09-.^\\ \xff\x80\x00Az9!"


def rs_text():
    r = rng.random()
    if r < 0.3:
        return bytes(rng.choice(RS_BYTES) for _ in range(rng.randrange(0, 8)))
    if r < 0.6:
        return bytes(rng.choice(b"az09ZA.-_ ") for _ in range(rng.randrange(0, 6))) + rng.choice([b"z", b"Z", b"9", b"zz", b"9z", b"z9", b"Zz", b".9", b"\xff",
                                                                                                    b"\xff\xff", b"", b"a"])
    return bytes(rng.randrange(256) for _ in range(rng.randrange(0, 6)))


RS_SET_PIECES = [b"a-z", b"A-Z", b"0-9", b"a", b"z", b"^", b"\\", b"-", b"\\-", b"\\^", b"a-a", b"\xff", b"\x80-\xff", b"aeiou", b" ", b"--", b"a-",
                 b"-a", b"^a-z", b"\\\\", b"!-/", b"b-d", b"^ ", b"o", b"l"]
RS_BAD_PIECES = [b"z-a", b"9-0", b"x-\\", b"\xff-\x80", b"a-A"]


def rs_set():
    pieces = [rng.choice(RS_SET_PIECES) for _ in range(rng.randrange(0, 4))]
    if rng.random() < 0.08:
        pieces.insert(rng.randrange(len(pieces) + 1), rng.choice(RS_BAD_PIECES))
    return b"".join(pieces)


def rs_subject():
    # Runs of one byte as often as not, so that squeezing has work to do.
    return b"".join(bytes([rng.choice(b"aabbzzoolll  09--^^\\\xff\x80AZ!")]) * rng.choice([1, 1, 2, 3]) for _ in range(rng.randrange(0, 8)))


def rubystring_line():
    h = lambda b: b.hex() or "-"
    r = rng.random()
    if r < 0.25:
        return f"succ {h(rs_text())}"
    if r < 0.55:
        return f"{rng.choice(['tr', 'tr', 'trs'])} {h(rs_subject())} {h(rs_set())} {h(rs_set() if rng.random() < 0.9 else b'')}"
    op = rng.choice(["del", "sq", "cnt"])
    return f"{op} {h(rs_subject())} " + " ".join(h(rs_set()) for _ in range(rng.choice([0, 1, 1, 1, 1, 2, 3]))).rstrip()


family("rubystring", "rubystring", [rubystring_line().rstrip() for _ in range(int(os.environ.get("RUBYSTRING_LINES", "8000")))],
       ["ruby", os.path.join(LIB, "rubystring/oracle.rb")])


# --- Node's StringDecoder, against Node 25 ----------------------------------------------------------------------

SD_NAMES = ["utf8", "utf-8", "UTF8", "UTF-8", "Utf8", "ucs2", "UCS2", "ucs-2", "utf16le", "UTF-16LE", "utf-16le", "latin1", "binary", "BINARY", "base64",
            "Base64", "base64url", "hex", "HEX", "ascii", "ASCII", "", "~", "utf16", "utf-16", "latin-1", "u8", "unicode", "base32", "İ"]


def sd_bytes():
    pieces = [b"a", b"\xc3\xa9", b"\xe2\x82\xac", b"\xf0\x9f\x98\x80", b"\xed\xa0\x80", b"\xc0\xaf", b"\xe0\x80\x80", b"\xf4\x90\x80\x80", b"\xff", b"\x80",
              b"\xbf\xbf\xbf\xbf", b"\xf8\x88\x80\x80", b"\x3d\xd8", b"\x00\xdc", b"\x41\x00", b"\xc3", b"\xe2\x82", b"\xf0\x9f", b"\xf0\x9f\x98", b"\x00"]
    return b"".join(rng.choice(pieces) for _ in range(rng.randrange(0, 6)))


def nodestringdecoder_line():
    name = rng.choice(SD_NAMES if rng.random() < 0.9 else ["~"])
    data = sd_bytes() + sd_bytes()
    cuts = sorted(rng.randrange(len(data) + 1) for _ in range(rng.randrange(0, 5)))
    steps, at = [], 0
    for c in cuts + [len(data)]:
        steps.append("w:" + (data[at:c].hex() or "-"))
        at = c
    if rng.random() < 0.3:
        last = steps.pop()
        steps.append("e:" + last[2:])
    else:
        steps.append("e")
    if rng.random() < 0.1:
        steps += ["w:" + (sd_bytes().hex() or "-"), "e"]
    enc = "~" if name == "~" else (name.encode().hex() or "-")
    return enc + " " + " ".join(steps)


family("nodestringdecoder", "nodestringdecoder", [nodestringdecoder_line() for _ in range(int(os.environ.get("NODESTRINGDECODER_LINES", "8000")))],
       ["node", os.path.join(LIB, "nodestringdecoder/oracle.js")])


# --- Python's http.cookies, against CPython 3.13 ---------------------------------------------------------------

CK_KEYS = ["a", "sid", "SID", "x-y", "k.1", "$Path", "$Domain", "$Foo", "$Version", "Path", "path", "Secure", "secure", "HttpOnly", "Max-Age", "max-age",
           "Comment", "expires", "SameSite", "a=b", "k,v", "a(b)", "a>b", "a@b", "q?", "{x}", "é", "a b", "", "1", "_"]
CK_VALUES = ["b", "", "\"quoted\"", "\"a\\\"b\"", "\"\\101\\102\"", "\"\\777\"", "\"a\\\nb\"", "\"open", "x y", "v,w", "a=b", "[1]", "Wed, 09 Jun 2021 10:18:14 GMT",
             "Wed, 09-Jun-2021 10:18:14 GMT", "Wed, 9 Jun 2021 10:18:14 GMT", "é", "/", "/a b", "\"\"", "\\", "\"\\\\\"", "x;y", "1", "None"]
CK_SEPS = ["; ", ";", " ", "  ", ";  ", "\t", " ; ", "\n", ";;"]


CK_PLAIN = ["a", "sid", "SID", "x-y", "k.1", "1", "_", "token", "q!"]
CK_ATTRS = ["Path", "path", "Domain", "Max-Age", "max-age", "Comment", "expires", "SameSite", "Version", "$Path", "$Domain", "$Version", "$Foo"]


def ck_item(first):
    r = rng.random()
    if first and r < 0.7 or not first and r < 0.4:
        return rng.choice(CK_PLAIN) + "=" + rng.choice(CK_VALUES)
    if not first and r < 0.75:
        a = rng.choice(CK_ATTRS)
        return a + rng.choice(["=", "=", " = "]) + rng.choice(CK_VALUES) if rng.random() < 0.9 else a
    if not first and r < 0.85:
        return rng.choice(["Secure", "HttpOnly", "secure", "httponly"])
    k = rng.choice(CK_KEYS)
    r = rng.random()
    if r < 0.15:
        return k
    if r < 0.25:
        return k + rng.choice([" = ", "= ", " =", "="]) + rng.choice(CK_VALUES)
    return k + "=" + rng.choice(CK_VALUES)


def ck_header():
    out = ""
    for n in range(rng.randrange(1, 6)):
        out += ck_item(n == 0) + rng.choice(CK_SEPS)
    if rng.random() < 0.5:
        out = out.rstrip()
    return rng.choice(["", " ", "\t"]) + out


def ck_hex(t):
    return t.encode().hex() or "-"


def pycookies_line():
    if rng.random() < 0.75:
        return f"ld {ck_hex(ck_header())}"
    key = rng.choice(["a", "sid", "x.y", "k!", "a b", "Path", "a=b", "é", ""])
    value = rng.choice(["v", "", "a b", "a\"b", "a\\b", "é", "€", "\x7f", "\x01", "a;b", "(x)", "{y}", "[z]", "x,y", "\u00ff\u0100"])
    parts = [ck_hex(key), ck_hex(value)]
    for _ in range(rng.randrange(0, 4)):
        attr = rng.choice(["path", "Path", "domain", "comment", "secure", "httponly", "max-age", "samesite", "version", "expires", "bogus", "PATH", "Comment"])
        v = rng.choice(["/", "", "x y", "Lax", "3600", "a\"b", "é", "~", "1"])
        parts += [ck_hex(key if rng.random() < 0.9 else "zz"), ck_hex(attr), v if v == "~" else ck_hex(v)]
    return "st " + " ".join(parts)


family("pycookies", "pycookies", [pycookies_line() for _ in range(int(os.environ.get("PYCOOKIES_LINES", "8000")))],
       ["python3.13", os.path.join(LIB, "pycookies/oracle.py")])


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
