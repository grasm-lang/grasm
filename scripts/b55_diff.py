"""B55's packages against the references they reproduce.

- SQLite 3.51.1's like() (with and without ESCAPE) and glob(), on values
  of every SQL type, errors included.

"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B55_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B55_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B55_LIB") or os.path.join(ROOT, "scripts/lib/b55")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b55")
harness = os.path.join(work, "b55test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b55test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


# --- SQLite's LIKE and GLOB, against SQLite 3.51.1 --------------------------------------------------------------

SL_ATOMS = ["a", "b", "A", "B", "x", "é", "É", "😀", "%", "_", "*", "?", "[a-c]", "[^a]", "[]x]", "[^]]", "[a-]", "[z-a]", "[é-ü]", "[", "]", "-", "\\",
            "!", "ab", "AB", "1", "\x7f", "日"]


def sl_word(atoms, n):
    return "".join(rng.choice(atoms) for _ in range(n))


def sl_value(t):
    r = rng.random()
    if r < 0.9:
        return "t" + (t.encode().hex() or "-")
    if r < 0.93:
        return "z"
    if r < 0.96:
        return "i" + str(rng.choice([12, 0, -5, 1234]))
    if r < 0.98:
        return "n" + struct.pack(">d", rng.choice([1.5, 0.1, 100.0])).hex()
    return "b" + (bytes(rng.randrange(0, 256) for _ in range(rng.randrange(0, 4))).hex() or "-")


SL_BAD = [b"\xc0\x80", b"\xed\xa0\x80", b"\xef\xbf\xbe", b"\x80", b"\xbf", b"\xf8\x88\x80\x80\x80", b"\xc3"]


def sl_pattern_char(ch):
    r = rng.random()
    if r < 0.6:
        return ch
    if r < 0.7:
        return ch.swapcase()
    if r < 0.8:
        return rng.choice(["_", "?", "%", "*"])
    lo, hi = ord(ch) - rng.choice([0, 1]), ord(ch) + rng.choice([0, 2])
    if lo < 33 or hi > 0x10FFFF or 0xD800 <= lo <= 0xDFFF or 0xD800 <= hi <= 0xDFFF:
        return "[" + ch + "]"
    return "[" + rng.choice(["", "^"]) + chr(lo) + "-" + chr(hi) + "]"


def sl_escaped(pat, esc):
    out = ""
    for ch in pat:
        if rng.random() < 0.2:
            out += esc + rng.choice(["%", "_", esc, "a"])
        out += ch
    return out


def sqlitelike_line():
    text_atoms = [a for a in SL_ATOMS if len(a) == 1 or a in ("ab", "AB")]
    text = sl_word(text_atoms, rng.randrange(0, 7))
    if rng.random() < 0.5:
        # A pattern built from the text, so matches are common.
        pat = "".join(sl_pattern_char(ch) for ch in text)
    else:
        pat = sl_word(SL_ATOMS, rng.randrange(0, 6))
    if rng.random() < 0.01:
        pat = "a" * 50001
    kind = rng.random()
    if rng.random() < 0.08:
        # Malformed UTF-8 in both, as BLOBs: SQLite reads it its own way.
        bad = rng.choice(SL_BAD)
        tb = text.encode() + bad + text[:1].encode()
        pb = rng.choice([bad, b"?", b"_", b"%", b"*"]) + (pat.encode() if rng.random() < 0.5 else b"") + rng.choice([b"", b"%", b"*", bad])
        return f"{'g' if kind < 0.5 else 'l'} b{pb.hex() or '-'} b{tb.hex() or '-'}"
    if kind < 0.4:
        return f"g {sl_value(pat)} {sl_value(text)}"
    if kind < 0.75:
        return f"l {sl_value(pat)} {sl_value(text)}"
    esc = rng.choice(["\\", "!", "%", "_", "x", "é", "", "ab", "z"])
    if len(esc) == 1 and rng.random() < 0.6:
        pat = sl_escaped(pat, esc)
    return f"l {sl_value(pat)} {sl_value(text)} " + ("z" if esc == "z" else "t" + (esc.encode().hex() or "-"))


family("sqlitelike", "sqlitelike", [sqlitelike_line() for _ in range(int(os.environ.get("SQLITELIKE_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "sqlitelike/oracle.py")])


# --- Node's path.win32, against Node 25 -------------------------------------------------------------------------

NW_PARTS = ["C:", "c:", "D:", "z:", "server", "share", "?", ".", "..", "...", "a", "b.txt", ".git", "x.tar.gz", "CON", "con", "con:", "COM1", "lpt²", "LPT³:",
            "NUL.txt", "AUX:x", "prn:", "é", "日本", "😀", "İX", "ΣΑΣ", ":", "C:foo", "UNC", "a.", ".a.b", "a:b", "x:", "CONx", "com1:"]
NW_SEPS = ["\\", "/", "\\\\", "//", "\\/", ""]
NW_CWDS = ["/Users/me/work", "C:\\Users\\me", "/", "c:\\", "\\\\srv\\sh\\dir", "/tmp/x/", "D:\\a\\b\\", "/home/İ/ΣΑΣ"]


def nw_path():
    r = rng.random()
    head = rng.choice(["", "", "", "\\", "/", "\\\\", "//", "\\\\?\\", "\\\\.\\", "C:\\", "c:", "\\\\srv\\sh", "//srv/sh/", "\\\\srv", "\\\\?\\C:\\", "\\\\.\\COM1:"])
    n = rng.randrange(0, 5)
    body = ""
    for k in range(n):
        body += rng.choice(NW_PARTS) + (rng.choice(NW_SEPS) if k < n - 1 or rng.random() < 0.3 else "")
    return head + body if r < 0.95 else rng.choice(["", ".", "\\", "/", "C:", "C:\\", "con", "CON:", "a"])


def nw_hex(t):
    return t.encode().hex() or "-"


def nodepathwin32_line():
    r = rng.random()
    if r < 0.15:
        return f"norm {nw_hex(nw_path())}"
    if r < 0.2:
        return f"abs {nw_hex(nw_path())}"
    if r < 0.32:
        return "join " + " ".join(nw_hex(nw_path()) for _ in range(rng.randrange(1, 4)))
    if r < 0.45:
        return f"res {nw_hex(rng.choice(NW_CWDS))} " + " ".join(nw_hex(nw_path()) for _ in range(rng.randrange(0, 4)))
    if r < 0.6:
        base = nw_path()
        to = base + rng.choice(["", "\\x", "/y\\z", "\\..", ""]) if rng.random() < 0.5 else nw_path()
        frm = base if rng.random() < 0.5 else nw_path()
        if rng.random() < 0.3:
            to = to.upper()
        return f"rel {nw_hex(rng.choice(NW_CWDS))} {nw_hex(frm)} {nw_hex(to)}"
    if r < 0.65:
        return f"ns {nw_hex(rng.choice(NW_CWDS))} {nw_hex(nw_path())}"
    if r < 0.72:
        return f"dir {nw_hex(nw_path())}"
    if r < 0.8:
        p = nw_path()
        if rng.random() < 0.5:
            return f"base {nw_hex(p)} {nw_hex(rng.choice(['.txt', 'txt', p[-3:], p, '', 'x', '.gz', 'b.txt']))}"
        return f"base {nw_hex(p)}"
    if r < 0.86:
        return f"ext {nw_hex(nw_path())}"
    if r < 0.95:
        return f"parse {nw_hex(nw_path())}"
    fields = [rng.choice(["", "", "C:\\", "\\", "/x/"]), rng.choice(["", "C:\\dir", "\\\\s\\h", "C:\\"]), rng.choice(["", "", "f.txt"]),
              rng.choice(["", "txt", ".gz", "."]), rng.choice(["", "name", "CON"])]
    return "fmt " + " ".join(nw_hex(x) for x in fields)


family("nodepathwin32", "nodepathwin32", [nodepathwin32_line() for _ in range(int(os.environ.get("NODEPATHWIN32_LINES", "10000")))],
       ["node", os.path.join(LIB, "nodepathwin32/oracle.js")])


# --- Ruby's String#inspect, #dump and #undump, against Ruby 2.6 --------------------------------------------------

RI_ATOMS = ["a", "Z", " ", "~", "\"", "\\", "#", "#{", "#$", "#@", "#x", "\n", "\t", "\r", "\x0b", "\x0c", "\x08", "\x07", "\x1b", "\x00", "\x01", "\x7f",
            "é", "日", "😀", "\u0085", " ", "​", "­", "", "͸", "\U0010ffff", "￾", "́", "؜", "\U000e0001",
            "\U0001f9ff", "๜", "ࣇ"]
RI_BAD = [b"\xff", b"\xc0\x80", b"\xed\xa0\x80", b"\xe6\x97", b"\xf4\x90\x80\x80", b"\x80", b"#\xff", b"#\xe6"]


def ri_bytes():
    out = b""
    for _ in range(rng.randrange(0, 7)):
        r = rng.random()
        if r < 0.65:
            out += rng.choice(RI_ATOMS).encode()
        elif r < 0.85:
            c = rng.randrange(0, 0x110000)
            if not 0xD800 <= c <= 0xDFFF:
                out += chr(c).encode()
        else:
            out += rng.choice(RI_BAD)
    return out


RI_ENC = ["utf-8", "UTF-8", "ASCII-8BIT", "binary", "Shift_JIS", "euc-jp", "US-ASCII", "ascii", "UTF-16LE", "locale", "external", "filesystem", "Utf8", "nope",
          "Windows-1252", "CP65001", "ISO-8859-1", "macRoman", ""]


def ri_dumped():
    body = ""
    for _ in range(rng.randrange(0, 6)):
        r = rng.random()
        if r < 0.4:
            body += rng.choice(["a", " ", "#", "~", "'"])
        elif r < 0.6:
            body += "\\" + rng.choice("nrtfvbae\\\"#qz0")
        elif r < 0.75:
            body += "\\u" + rng.choice(["00E9", "0041", "D800", "12", "1F60", "FFFF", "{1F600}", "{41 42}", "{}", "{110000}", "{DFFF}", "{1234567}", "{ 41}",
                                        "{41", "zz"])
        else:
            body += "\\x" + rng.choice(["FF", "41", "0", "zz", "e9", ""])
    t = '"' + body + '"'
    r = rng.random()
    if r < 0.15:
        t += rng.choice([".force_encoding(\"", ".dup.force_encoding(\""]) + rng.choice(RI_ENC) + "\")"
    elif r < 0.2:
        t += rng.choice([".force_encoding(\"utf-8\"", ".force_encoding(utf-8)", ".dup", "x", ".force_encoding(\"utf-8\"))"])
    elif r < 0.25:
        t = rng.choice([t[:-1], t[1:], "\"", "", "\"aé\"", "\"a\x00\"", t + "\\"])
    return t


def rubyinspect_line():
    r = rng.random()
    if r < 0.45:
        return f"i {ri_bytes().hex() or '-'} {int(rng.random() < 0.3)}"
    if r < 0.8:
        return f"d {ri_bytes().hex() or '-'} {int(rng.random() < 0.3)}"
    return f"u {ri_dumped().encode().hex() or '-'}"


family("rubyinspect", "rubyinspect", [rubyinspect_line() for _ in range(int(os.environ.get("RUBYINSPECT_LINES", "10000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubyinspect/oracle.rb")])


# --- Ruby's to_i, to_f, Integer() and Float(), against Ruby 2.6 -------------------------------------------------

RN_ATOMS = ["0", "1", "7", "9", "12", "00", "_", "__", " ", "\t", "\n", "+", "-", "0x", "0X", "0b", "0o", "0d", "0B", "0O", "0D", "a", "f", "F", "z", "Z", ".",
            "e", "E", "e+", "e-", "p", "P", "p-", "1e5", "1.5", ".5", "5.", "0x1.8p1", "x", "é", "\x00", "1_000", "inf", "NaN", "0.0", "1e400", "1e-400",
            "123456789", "9223372036854775807", "18446744073709551616"]


def rn_halfway():
    """A point halfway between two doubles, written out exactly, with a
    tail that decides the rounding only if it is read: Ruby reads 60
    significant digits of a fraction."""
    from decimal import Decimal, getcontext
    getcontext().prec = 800
    x = rng.uniform(0.5, 2.0) * 2.0 ** rng.choice([0, -3, -10, 3])
    nxt = struct.unpack("<d", struct.pack("<Q", struct.unpack("<Q", struct.pack("<d", x))[0] + 1))[0]
    mid = (Decimal(x) + Decimal(nxt)) / 2
    text = format(mid, "f")
    return text + rng.choice(["", "0" * rng.randrange(1, 20) + "1", "000001", "9"]) + rng.choice(["", "", "x", " "])


def rn_special():
    r = rng.random()
    if r < 0.4:
        return rn_halfway()
    if r < 0.55:
        return "0x" + rng.choice(["1", "0", "f"]) + "." + "".join(rng.choice("0123456789abcdef") for _ in range(rng.randrange(260, 300))) + rng.choice(["p0", "p-3", "p1"])
    if r < 0.75:
        return "0x" + rng.choice(["0.8", "1.8", "2.8", "3.8", "1.4", "1.c", "0.c", "2.4", "5.8"]) + "p" + str(rng.choice([-1074, -1075, -1073, -1076, -1077]))
    return rng.choice(["", "-", " ", "+"]) + rng.choice(["0__1", "0_0_1", "0__", "00__1", "0_1", "0x_1", "0_x1", "0___1", "0b__1", "0_0__1"])


def rn_text():
    r = rng.random()
    if r < 0.12:
        return rn_special()
    if r < 0.08:
        return rng.choice(["1", "0", "-", "", " "]) + "".join(rng.choice("0123456789") for _ in range(rng.randrange(55, 85))) + rng.choice(["", "x", " ", "_1", ".5",
                                                                                                                                            "\x00", "e3"])
    if r < 0.14:
        return rng.choice(["0.", "1.", "-0.", ""]) + "".join(rng.choice("0123456789") for _ in range(rng.randrange(55, 85))) + rng.choice(["", "x", " ", "e-5"])
    if r < 0.2:
        return rng.choice(["0x", "-0x", " 0x", "+0x", "0X"]) + "".join(rng.choice("0123456789abcdef") for _ in range(rng.randrange(1, 20))) + rng.choice(
            ["", ".8", ".", "p", "p4", "p-1074", ".0p0", "p+10", "p2100", "p-2100", "x", " "])
    return "".join(rng.choice(RN_ATOMS) for _ in range(rng.randrange(0, 6)))


def rubynumparse_line():
    t = rn_text().encode().hex() or "-"
    r = rng.random()
    if r < 0.3:
        base = rng.choice([10, 10, 10, 16, 2, 8, 0, 36, rng.randrange(2, 37), 1, 37, -1])
        return f"ti {t} {base}"
    if r < 0.55:
        base = rng.choice(["-", "-", "-", "10", "16", "2", "8", "0", "36", "1", "37", "-16", "-2"])
        return f"ii {t} {base}"
    if r < 0.78:
        return f"tf {t}"
    return f"ff {t}"


family("rubynumparse", "rubynumparse", [rubynumparse_line() for _ in range(int(os.environ.get("RUBYNUMPARSE_LINES", "12000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubynumparse/oracle.rb")])


# --- Lua's string.pack, unpack and packsize, against Lua 5.5 ----------------------------------------------------

LP_OPTS = ["b", "B", "h", "H", "l", "L", "j", "J", "T", "f", "d", "n", "i", "I", "i1", "i2", "i3", "i4", "i7", "i8", "i9", "i12", "i16", "I3", "I8", "I9", "I16",
           "s", "s1", "s2", "s4", "s8", "z", "c3", "c0", "c1", "x", "Xi4", "Xd", "Xi16", "Xc2", "X", " ", "<", ">", "=", "!", "!2", "!4", "!8", "!16", "!3",
           "i0", "i17", "c", "q", "Xx", "X<"]


def lp_fmt():
    return "".join(rng.choice(LP_OPTS) for _ in range(rng.randrange(0, 5)))


def lp_value():
    r = rng.random()
    if r < 0.45:
        return "i" + str(rng.choice([0, 1, -1, 127, 128, -128, -129, 255, 256, 65535, 65536, -32768, 2 ** 31, -2 ** 31, 2 ** 32, 2 ** 63 - 1, -2 ** 63,
                                     rng.randrange(-10 ** 6, 10 ** 6)]))
    if r < 0.65:
        v = rng.choice([0.0, -0.0, 1.5, 1e300, 3.4028235e38, 3.4028236e38, 1e-45, 1.4e-45, 7e-46, 0.1, float("inf"), -float("inf"), 2.0 ** 63, 100.0,
                        float("nan"), 1e-40, 16777217.0, 16777219.0, 1 + 3 * 2.0 ** -24, -(1 + 3 * 2.0 ** -24), 3 * 2.0 ** -150, 2.0 ** -150])
        return "d" + struct.pack(">d", v).hex()
    if r < 0.9:
        return "s" + (rng.choice([b"", b"abc", b"a\x00b", b"10", b" 0x10 ", b"1e2", b"1.5", b"x" * 300, b"\xff\xfe", b"-7"]).hex() or "-")
    return rng.choice(["n", "t", "f", "T"])


def luapack_line():
    if rng.random() < 0.04:
        # Doubles that fall between two floats, for 'f'.
        v = rng.choice([16777217.0, 16777219.0, 1 + 3 * 2.0 ** -24, 1 + 2.0 ** -24, 3 * 2.0 ** -150, 2.0 ** -150, 5 * 2.0 ** -150, 3.4028235677973366e38,
                        rng.uniform(-1e6, 1e6)])
        return f"p {rng.choice(['f', '>f', '<f', 'ff']).encode().hex()} d{struct.pack('>d', v).hex()} d{struct.pack('>d', -v).hex()}"
    r = rng.random()
    fmt = lp_fmt()
    fh = fmt.encode().hex() or "-"
    if r < 0.45:
        return " ".join(["p", fh] + [lp_value() for _ in range(rng.randrange(0, 5))])
    if r < 0.6:
        return f"z {fh}"
    data = bytes(rng.randrange(0, 256) if rng.random() < 0.7 else rng.choice([0, 255, 1]) for _ in range(rng.randrange(0, 40)))
    init = rng.choice([1, 1, 1, 2, 0, -1, -5, 100, len(data) + 1, len(data) + 2, -100])
    return f"u {fh} {data.hex() or '-'} {init}"


family("luapack", "luapack", [luapack_line() for _ in range(int(os.environ.get("LUAPACK_LINES", "12000")))], ["lua", os.path.join(LIB, "luapack/oracle.lua")])


# --- jq's @-formats, against jq 1.7.1 --------------------------------------------------------------------------

JQ_STRINGS = ['""', '"a b"', '"x\\"y"', '"it\'s"', '"tab\\there"', '"nl\\nx"', '"\\u0000"', '"\\\\"', '"<a href=\\"x\\">&</a>"', '"é/~*+!"', '"😀"',
              '"\\u007f\\u0001"', '"\\ud83d\\ude00"', '"YWJj"', '"YWJ"', '"Y"', '"YQ=="', '"!!!"', '"aGVsbG8gd29ybGQ="', '"/w=="', '"wA=="',
              '"7Q=="', '"7aCA"', '"8J+Y"', '"YWJj=xyz"', '"YW Jj"', '"\\u2028"', '"%41"', '"a,b"', '"\\r\\n"']
JQ_NUMBERS = ["0", "-0", "1", "1.5", "1e2", "1E2", "1.0e3", "100", "0.0000001", "0.000001", "123.456e-2", "-1.0", "1.5e300", "0e10", "-0.0e-5", "10.50e1",
              "0.001e-3", "123456789012345678901234567890", "3.0", "1e-400", "1E+400", "0.10", "2.5e-7", "1e-6"]


def jq_value(depth=0):
    r = rng.random()
    if r < 0.35 or depth > 2:
        return rng.choice(JQ_STRINGS)
    if r < 0.6:
        return rng.choice(JQ_NUMBERS)
    if r < 0.7:
        return rng.choice(["null", "true", "false"])
    if r < 0.9:
        return "[" + ",".join(jq_value(depth + 1) for _ in range(rng.randrange(0, 4))) + "]"
    keys = rng.sample(['"a"', '"b"', '"long key here"', '"é"', '"a"'], rng.randrange(0, 3))
    return "{" + ",".join(k + ":" + jq_value(depth + 1) for k in keys) + "}"


def jqformat_line():
    fmt = rng.choice(["text", "json", "html", "uri", "csv", "tsv", "sh", "base64", "base64d", "csv", "tsv", "sh", "base64d", "base32", "nope"])
    v = jq_value()
    if fmt in ("csv", "tsv", "sh") and rng.random() < 0.6:
        v = "[" + ",".join(rng.choice([rng.choice(JQ_STRINGS), rng.choice(JQ_NUMBERS), "null", "true", "false", "[1]", '{"a":"bbbbbbbbbbbbbbbbbb"}'])
                           for _ in range(rng.randrange(0, 5))) + "]"
    return f"jq {fmt} {v.encode().hex()}"


family("jqformat", "jqformat", [jqformat_line() for _ in range(int(os.environ.get("JQFORMAT_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "jqformat/oracle.py")])


# --- Lua's utf8 library, against Lua 5.5 ------------------------------------------------------------------------

LU_PIECES = [b"a", b"\xc3\xa9", b"\xe6\x97\xa5", b"\xf0\x9f\x98\x80", b"\xed\xa0\x80", b"\xf4\x90\x80\x80", b"\xf8\x88\x80\x80\x80", b"\xfc\x84\x80\x80\x80\x80",
             b"\xfd\xbf\xbf\xbf\xbf\xbf", b"\xc0\x80", b"\xe0\x80\xaf", b"\x80", b"\xbf", b"\xff", b"\xfe", b"\xc3", b"\xe6\x97", b"\x00", b"\xef\xbf\xbf",
             b"\xe0\x9f\xbf", b"\xe0\xa0\x80", b"\xfe\x80\x80\x80\x80\x80\x80", b"\xf0\x8f\xbf\xbf", b"\xc1\xbf"]


def lu_text():
    return b"".join(rng.choice(LU_PIECES) for _ in range(rng.randrange(0, 6)))


def lu_pos(n):
    return str(rng.choice([1, 2, 3, -1, -2, 0, n, n + 1, n + 2, -n, -n - 1, rng.randrange(-n - 2, n + 3)]))


def luautf8_line():
    r = rng.random()
    t = lu_text()
    h = t.hex() or "-"
    lax = rng.choice("TF")
    if r < 0.15:
        return "luautf8 char " + " ".join(str(rng.choice([0, 65, 233, 0x7ff, 0x800, 0xffff, 0x10000, 0x10ffff, 0x110000, 0xd800, 0x7fffffff, 0x80000000, -1,
                                                           0x3ffffff, 0x4000000, 0x1fffff, 0x200000])) for _ in range(rng.randrange(1, 4)))
    if r < 0.35:
        return f"luautf8 codes {h} {lax}"
    if r < 0.55:
        return f"luautf8 codepoint {h} {lu_pos(len(t))} {rng.choice(['_', lu_pos(len(t))])} {lax}"
    if r < 0.8:
        return f"luautf8 len {h} {rng.choice(['1', lu_pos(len(t))])} {rng.choice(['-1', lu_pos(len(t))])} {lax}"
    return f"luautf8 offset {h} {rng.choice([0, 1, 2, 3, -1, -2, -3, 5, 10, -10])} {rng.choice(['_', lu_pos(len(t))])}"


family("luautf8", "luautf8", [luautf8_line() for _ in range(int(os.environ.get("LUAUTF8_LINES", "10000")))], ["lua", os.path.join(LIB, "luautf8/oracle.lua")])


# --- Java's Integer and Long text conversions, against Java 17 ---------------------------------------------------

JI_ATOMS = ["0", "1", "7", "9", "12", "a", "f", "z", "Z", "F", "-", "+", "x", "0x", "0X", "#", "00", " ", "١٢٣", "٠", "५", "７", "ｆ", "Ａ", "２",
            "2147483647", "2147483648", "-2147483648", "4294967295", "4294967296", "9223372036854775807", "9223372036854775808",
            "18446744073709551615", "18446744073709551616", "ffffffff", "7fffffff", "80000000", "zzzzzzzzzzzzz", "3w5e11264sgsf", "😀", "_"]


def ji_text():
    return "".join(rng.choice(JI_ATOMS) for _ in range(rng.randrange(0, 4)))


def javaintegers_line():
    r = rng.random()
    if r < 0.6:
        op = rng.choice(["pi", "pl", "pui", "pul", "pi", "pl", "pul"])
        radix = rng.choice([10, 10, 10, 16, 2, 8, 36, rng.randrange(2, 37), 1, 37, 0, -10])
        return f"javaintegers {op} {ji_text().encode().hex() or '-'} {radix}"
    if r < 0.75:
        return f"javaintegers {rng.choice(['di', 'dl'])} {ji_text().encode().hex() or '-'}"
    v = rng.choice([0, 1, -1, 255, -255, 2 ** 31 - 1, -2 ** 31, 2 ** 31, 2 ** 32 - 1, 2 ** 63 - 1, -2 ** 63, rng.randrange(-2 ** 63, 2 ** 63),
                    rng.randrange(-2 ** 31, 2 ** 31)])
    radix = rng.choice([10, 16, 2, 8, 36, 3, 32, 4, 1, 37, rng.randrange(2, 37)])
    return f"javaintegers {rng.choice(['ts', 'tus', 'th', 'to', 'tb', 'ts', 'tus'])} {v} {radix} {rng.choice([32, 64])}"


family("javaintegers", "javaintegers", [javaintegers_line() for _ in range(int(os.environ.get("JAVAINTEGERS_LINES", "10000")))],
       ["java", "-cp", JAVA, "IntegersOracle"])


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
