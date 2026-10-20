"""B52's packages against the references they reproduce.

- Go 1.25's path (Clean, Join, Split, Base, Dir, Ext, IsAbs, Match) and,
  on Unix, filepath.Join, Rel and SplitList.
- Go 1.25's http.DetectContentType.
- CPython 3.13's csv: reader and writer with every dialect option and
  quoting mode, the errors, and float() for the numeric modes.
- Java 17's java.time.Duration and Period: parse, toString and arithmetic,
  with Java's exceptions.
- Node 25's util.format and util.inspect on primitive values.
- CPython 3.13's posixpath: normpath, join, split, splitext, splitroot,
  basename, dirname, isabs, commonprefix, commonpath, abspath and relpath.
- Ruby 2.6's Shellwords (split, escape, join) and String#inspect; the gate
  also checks the package's generated printable table is current.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B52_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B52_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B52_LIB") or os.path.join(ROOT, "scripts/lib/b52")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b52")
harness = os.path.join(work, "b52test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b52test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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
for f in os.listdir(os.path.join(LIB, "java")):
    if f.endswith(".java"):
        shutil.copy(os.path.join(LIB, "java", f), JAVA)
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


# --- Go's path, against Go 1.25 -------------------------------------------------------------------------------

GP_SEGS = ["a", "b", "c.d", ".", "..", "...", ".x", "x.", "", "", "é", "a b", "\\", "*", "?", "[", "]"]


def gp_path():
    t = rng.choice(["", "", "/", "//", "./", "../"]) + "/".join(rng.choice(GP_SEGS) for _ in range(rng.randrange(0, 5)))
    return t + rng.choice(["", "", "/", "//"])


GP_PATTERN = ["a", "b", "*", "*", "?", "[a-c]", "[^a]", "[!a]", "\\*", "\\", "[", "[]", "[a-]", "[-a]", "[\\]]", "[a-b-c]", "/", ".", "é", "[é-ü]", "**",
              "x", "[^/]", "[\\-]", "[a", "c.d", "\\a"]


def gp_hex(t):
    return t.encode().hex() or "-"


def gp_list():
    items = [gp_path() for _ in range(rng.randrange(0, 4))]
    return ",".join(gp_hex(x) for x in items) if items else "~"


def gopath_line():
    r = rng.random()
    if r < 0.12:
        return f"cl {gp_hex(gp_path())}"
    if r < 0.2:
        return f"jo {gp_list()}"
    if r < 0.26:
        return f"fj {gp_list()}"
    if r < 0.3:
        return f"sp {gp_hex(gp_path())}"
    if r < 0.35:
        return f"ex {gp_hex(gp_path())}"
    if r < 0.4:
        return f"ba {gp_hex(gp_path())}"
    if r < 0.45:
        return f"di {gp_hex(gp_path())}"
    if r < 0.47:
        return f"ia {gp_hex(gp_path())}"
    if r < 0.8:
        pat = "".join(rng.choice(GP_PATTERN) for _ in range(rng.randrange(0, 5)))
        name = gp_path() if rng.random() < 0.4 else "".join(rng.choice(["a", "b", "c", "/", ".", "d", "*", "é", "-", "]"]) for _ in range(rng.randrange(0, 6)))
        return f"ma {gp_hex(pat)} {gp_hex(name)}"
    if r < 0.95:
        return f"re {gp_hex(gp_path())} {gp_hex(gp_path())}"
    return f"sl {gp_hex(':'.join(rng.choice(['', 'a', '/b', 'c:d']) for _ in range(rng.randrange(0, 4))))}"


gopath_bin = go_tool("gopath", os.path.join(LIB, "gopath/oracle.go"))
family("gopath", "gopath", [gopath_line() for _ in range(int(os.environ.get("GOPATH_LINES", "8000")))], [gopath_bin])


# --- Go's DetectContentType, against Go 1.25 -------------------------------------------------------------------

SN_PREFIXES = [b"<!DOCTYPE HTML", b"<!doctype html", b"<HTML", b"<html", b"<HeAd", b"<script", b"<IFRAME", b"<h1", b"<div", b"<font", b"<table", b"<a",
               b"<style", b"<title", b"<b", b"<body", b"<br", b"<p", b"<!--", b"<?xml", b"<?XML", b"%PDF-", b"%!PS-Adobe-", b"\xfe\xff", b"\xff\xfe",
               b"\xef\xbb\xbf", b"\x00\x00\x01\x00", b"\x00\x00\x02\x00", b"BM", b"GIF87a", b"GIF89a", b"RIFF\x10\x00\x00\x00WEBPVP8", b"\x89PNG\r\n\x1a\n",
               b"\xff\xd8\xff", b"FORM\x00\x00\x00\x00AIFF", b"ID3", b"OggS\x00", b"MThd\x00\x00\x00\x06", b"RIFF\x00\x00\x00\x00AVI ",
               b"RIFF\x00\x00\x00\x00WAVE", b"\x1a\x45\xdf\xa3", b"\x00\x01\x00\x00", b"OTTO", b"ttcf", b"wOFF", b"wOF2", b"\x1f\x8b\x08", b"PK\x03\x04",
               b"Rar!\x1a\x07\x00", b"Rar!\x1a\x07\x01\x00", b"\x00asm", b"plain text", b"", b"<", b"<?", b"%PDF", b"Rar!\x1a\x07"]


def sn_mp4():
    brands = [rng.choice([b"isom", b"mp41", b"mp42", b"avc1", b"qt  ", b"mp4x"]) for _ in range(rng.randrange(0, 5))]
    body = b"ftyp" + rng.choice([b"isom", b"mp42", b"mp4 "]) + rng.choice([b"\x00\x00\x02\x00", b"mp41"]) + b"".join(brands)
    size = rng.choice([4 + len(body), 4 + len(body) + rng.choice([0, 0, 1, 2, 4, 100]), 8])
    return size.to_bytes(4, "big") + body


def sn_data():
    r = rng.random()
    if r < 0.12:
        d = sn_mp4()
    elif r < 0.17:
        d = b"\x00" * 34 + b"LP" + bytes(rng.randrange(256) for _ in range(rng.randrange(0, 4)))
    else:
        d = rng.choice(SN_PREFIXES)
        if rng.random() < 0.2 and d:
            i = rng.randrange(len(d))
            d = d[:i] + bytes([rng.randrange(256)]) + d[i + 1:]
    tail = rng.choice([b"", b" ", b">", b"x", b"\x00", b"\x1b", b"\x0b", b"hello\n", bytes(rng.randrange(256) for _ in range(rng.randrange(0, 8)))])
    pre = rng.choice([b"", b"", b"", b" ", b"\n\t", b"\x0c\r "])
    data = pre + d + tail
    if rng.random() < 0.05:
        data = data + b"a" * rng.choice([500, 520]) + rng.choice([b"\x00", b""])
    return data


family("gosniff", "gosniff", [f"sn {sn_data().hex() or '-'}" for _ in range(int(os.environ.get("GOSNIFF_LINES", "6000")))],
       [go_tool("gosniff", os.path.join(LIB, "gosniff/oracle.go"))])


# --- Python's csv, against CPython 3.13 -------------------------------------------------------------------------

PC_FIELDS = ["a", "bc", "1", "2.5", "-3", "1e5", "1_000", "inf", "nan", "x y", " lead", "", "", '"q"', 'a"b', "a,b", "a;b", "line\nbreak", "cr\rlf",
             "tab\t", "é", "١٢", " 7 ", "\\", "\\,", "'", "|", "$", "1.", ".5", "0x10", "--1", "a\\\nb", "x\\\r", "\\\r\ny", "$\nz"]


def pc_dialect():
    delim = rng.choice([44, 44, 44, 59, 9, 124, 32, 34, 10, 13])
    quote = rng.choice(["34", "34", "34", "39", "~", "124", "44"])
    esc = rng.choice(["~", "~", "~", "92", "36", "34"])
    term = rng.choice(["\r\n", "\r\n", "\n", "\r", "|", "\n\n", "x"])
    return "/".join([str(delim) if rng.random() < 0.97 else "~", quote, esc, rng.choice("110"), rng.choice("001"), term.encode().hex() or "-",
                     str(rng.choice([0, 0, 1, 2, 3, 4, 5, 0, 1, 6])), rng.choice("001")])


def pc_text():
    rows = []
    for _ in range(rng.randrange(0, 4)):
        fields = []
        for _ in range(rng.randrange(1, 4)):
            f = rng.choice(PC_FIELDS)
            r = rng.random()
            if r < 0.3:
                f = '"' + f.replace('"', '""') + '"'
            elif r < 0.35:
                f = '"' + f
            elif r < 0.4:
                f = "'" + f + "'"
            fields.append(rng.choice(["", "", " "]) + f)
        rows.append(rng.choice([",", ",", ";", "\t", "|", " "]).join(fields))
    ends = [rng.choice(["\n", "\r\n", "\r", "\n"]) for _ in rows]
    t = "".join(r + e for r, e in zip(rows, ends))
    if t and rng.random() < 0.3:
        t = t[:-1]
    if rng.random() < 0.05:
        t += rng.choice(["\n\n", "\r\r\n", "\\\n", '"\n'])
    return t


def pc_field():
    r = rng.random()
    if r < 0.6:
        return "s" + (rng.choice(PC_FIELDS).encode().hex() or "")
    if r < 0.75:
        return "i" + str(rng.choice([0, 1, -5, 12345678901234567890]))
    if r < 0.9:
        return "d%x" % dbits(rng.choice([1.5, -0.0, 1e21, 0.1, float("inf"), float("nan"), 1e-7, 3.0]))
    return "n"


def pycsv_line():
    r = rng.random()
    if r < 0.5:
        return f"rd {pc_text().encode().hex() or '-'} {pc_dialect()}"
    if r < 0.85:
        fields = [pc_field() for _ in range(rng.randrange(0, 4))]
        return f"wr {','.join(fields) or '~'} {pc_dialect()}"
    t = rng.choice(["1", " 2.5 ", "1_000", "1__0", "_1", "1_", "inf", "-Infinity", "NaN", "+nan", "1e5", "1e", ".", ".5", "5.", "١٢.٥", "　 3 ",
                    "0x10", "1,5", "", " ", "1e1_0", "é", "1.5e-3", "-0", "infinit", "1_000.000_1", "12e_3"])
    return f"fl {t.encode().hex() or '-'}"


def pycsv_lines():
    lines = [pycsv_line() for _ in range(int(os.environ.get("PYCSV_LINES", "20000")))]
    # The field limit, once.
    lines.append(f"rd {('a' * 131073).encode().hex()} 44/34/~/1/0/{'\r\n'.encode().hex()}/0/0")
    return lines


family("pycsv", "pycsv", pycsv_lines(), ["python3.13", os.path.join(LIB, "pycsv/oracle.py")], chunk=200)


# --- Java's Duration and Period, against Java 17 ----------------------------------------------------------------

LMAX, LMIN, IMAX, IMIN = (1 << 63) - 1, -(1 << 63), (1 << 31) - 1, -(1 << 31)


def jd_text():
    r = rng.random()
    if r < 0.1:
        return rng.choice(["PT0S", "P0D", "PT", "P", "pt1h", "P1Dt", "P1DT", "PT1.5S", "PT-0.5S", "-PT6H3M", "+P2D", "PT1,5S", "PT1.S", "PT.5S", "P1W",
                           "PT1.1234567890S", "PT1H1H", "PT1M1H", "P-1DT-1H", "p2dt3h4m5.6s", "PT9223372036854775807S", "PT9223372036854775808S",
                           "P106751991167301D", "P106751991167300DT24H", "-PT-9223372036854775808S", "PT-9223372036854775808S", " PT1S", "PT1S ", "PT١S"])
    num = lambda: rng.choice(["", "-", "+"]) + rng.choice(["0", "1", "5", "60", "3600", "86400", str(rng.randrange(10 ** rng.randrange(1, 20)))])
    t = rng.choice(["", "", "-", "+"]) + rng.choice(["P", "P", "p"])
    if rng.random() < 0.5:
        t += num() + rng.choice("Dd")
    if rng.random() < 0.8:
        t += rng.choice("Tt")
        if rng.random() < 0.5:
            t += num() + rng.choice("Hh")
        if rng.random() < 0.5:
            t += num() + rng.choice("Mm")
        if rng.random() < 0.6:
            t += num()
            if rng.random() < 0.4:
                t += rng.choice(".,") + "".join(rng.choice("0123456789") for _ in range(rng.randrange(0, 11)))
            t += rng.choice("Ss")
    return t


def jp_text():
    if rng.random() < 0.1:
        return rng.choice(["P0D", "P", "P1Y2M3W4D", "p1y", "-P1Y", "+P-1M", "P2147483647D", "P2147483648D", "P-2147483648D", "-P-2147483648D", "P306783379W",
                           "P306783378W1D", "P1D1W", "PT1H", "P1Y1Y", "P1.5D"])
    num = lambda: rng.choice(["", "-", "+"]) + rng.choice(["0", "1", "12", "13", "100", str(rng.randrange(10 ** rng.randrange(1, 11)))])
    t = rng.choice(["", "", "-", "+"]) + rng.choice(["P", "p"])
    for letter in "YMWD":
        if rng.random() < 0.45:
            t += num() + rng.choice([letter, letter.lower()])
    return t


def jlong():
    return rng.choice([0, 1, -1, 59, 3600, -3600, 86400, LMAX, LMIN, LMAX // 1000, rng.randrange(-10 ** 12, 10 ** 12), rng.randrange(LMIN, LMAX)])


def jnano():
    return rng.choice([0, 1, 500000000, 999999999, 1000000000, -1, -1500000000, rng.randrange(-3 * 10 ** 9, 3 * 10 ** 9)])


def jint():
    return rng.choice([0, 1, -1, 12, 13, -13, 7, IMAX, IMIN, rng.randrange(-1000, 1000), rng.randrange(IMIN, IMAX)])


def javaduration_line():
    r = rng.random()
    if r < 0.3:
        return f"dp {jd_text().encode().hex()}"
    if r < 0.38:
        return f"dt {jlong()} {jnano()}"
    if r < 0.65:
        k = rng.choice(["plus", "mul", "div", "divd", "neg", "millis", "nanos"])
        s, n = rng.choice([(jlong(), jnano()), (rng.randrange(-10 ** 6, 10 ** 6), rng.randrange(10 ** 9))])
        if k in ("plus", "divd"):
            return f"da {k} {s} {n} {jlong() if rng.random() < 0.3 else rng.randrange(-10 ** 5, 10 ** 5)} {jnano()}"
        if k in ("mul", "div"):
            return f"da {k} {s} {n} {rng.choice([0, 1, -1, 2, 3, 1000, LMAX, LMIN, rng.randrange(-10 ** 9, 10 ** 9)])}"
        return f"da {k} {s} {n}"
    if r < 0.85:
        return f"pp {jp_text().encode().hex()}"
    k = rng.choice(["plus", "mul", "norm", "id"])
    return f"pa {k} {jint()} {jint()} {jint()} {jint()} {jint()} {jint()}"


family("javaduration", "javaduration", [javaduration_line() for _ in range(int(os.environ.get("JAVADURATION_LINES", "8000")))],
       ["java", "-cp", JAVA, "DurationOracle"])


# --- Node's util.format, against Node 25 ------------------------------------------------------------------------

UF_PIECES = ["%s", "%d", "%i", "%f", "%j", "%o", "%O", "%c", "%%", "%x", "%", "a", " ", "b c", "\n", "'", '"', "`", "${", "\\", "\t", "\x0b", "\x7f", "\x85",
             " ", "é", "😀", "\ud800", "\udc00", "%%s", "x%"]
UF_STRINGS = ["", "a", "it's", 'say "hi"', "both ' and \"", "back`tick'\"", "${x}'\"", "0x10", "  42  ", "42.9px", "1e3", "-0", "Infinity", "NaN", "\n",
              "line1\nline2\n", "😀", "\ud800x", "\x00\x1b", "\x85\x9f\xa0", "true", "null"]


def uf_units(s):
    out = []
    for ch in s:
        c = ord(ch)
        if c >= 0x10000:
            out += [0xD800 + ((c - 0x10000) >> 10), 0xDC00 + ((c - 0x10000) & 0x3FF)]
        else:
            out.append(c)
    return ".".join("%x" % u for u in out) or "-"


def uf_arg():
    r = rng.random()
    if r < 0.4:
        s = rng.choice(UF_STRINGS)
        if rng.random() < 0.1:
            s = rng.choice(["a" * 80 + "\n" + "b" * 10, ("x\n" * 50), "y" * 100, "q'" * 60 + "\n", "c" * 40 + "\n" + "d" * 37, "e" * 76 + "\n", "f\n" * 39])
        return "s" + uf_units(s)
    if r < 0.65:
        return "n%x" % dbits(rng.choice([0.0, -0.0, 1.0, -1.5, 1e21, 1e-7, 0.1, float("nan"), float("inf"), -float("inf"), 42.0, 16.0, 2 ** 53, 123.456]))
    if r < 0.72:
        return "b" + str(rng.choice([0, 1, -5, 2 ** 70, -(2 ** 64)]))
    return rng.choice(["t", "f", "l", "u"])


def nodeutilformat_line():
    if rng.random() < 0.15:
        return f"in {uf_arg()}"
    args = []
    if rng.random() < 0.85:
        args.append("s" + uf_units("".join(rng.choice(UF_PIECES) for _ in range(rng.randrange(0, 7)))))
    args += [uf_arg() for _ in range(rng.randrange(0, 4))]
    return f"fm {','.join(args) or '~'}"


def nodeutilformat_lines():
    lines = [nodeutilformat_line() for _ in range(int(os.environ.get("NODEUTILFORMAT_LINES", "6000")))]
    lines.append("in s" + uf_units("z" * 10005))
    return lines


family("nodeutilformat", "nodeutilformat", nodeutilformat_lines(), ["node", os.path.join(LIB, "nodeutilformat/oracle.js")])


# --- Python's posixpath, against CPython 3.13 -------------------------------------------------------------------

PP_SEGS = ["a", "b", "c.d", ".x", "..y", "z.", "a.b.c", ".", "..", "...", "é", "😀", "\\", " ", "usr", "lib", "local", "", ""]


def pp_path():
    t = rng.choice(["", "", "/", "//", "///", "./", "../"]) + "/".join(rng.choice(PP_SEGS) for _ in range(rng.randrange(0, 5)))
    return t + rng.choice(["", "", "/", "//"])


def pp_hex(t):
    return t.encode().hex() or "-"


def pp_list(n):
    items = [pp_path() for _ in range(n)]
    return ",".join(pp_hex(x) for x in items) if items else "~"


def pyposixpath_line():
    cwd = pp_hex(rng.choice(["/", "/home/u", "/a/b/", "//srv", "/x/../y", "/é"]))
    op = rng.choice(["np", "np", "jo", "sp", "se", "se", "sr", "bn", "dn", "ia", "cp", "cm", "cm", "ab", "rp", "rp"])
    if op == "jo":
        return f"jo {cwd} {pp_list(rng.randrange(0, 4))}"
    if op == "cp":
        return f"cp {cwd} {pp_list(rng.randrange(0, 4))}"
    if op == "cm":
        return f"cm {cwd} {pp_list(rng.randrange(0, 4))}"
    if op == "rp":
        return f"rp {cwd} {pp_hex(pp_path())} {pp_hex(pp_path())}"
    return f"{op} {cwd} {pp_hex(pp_path())}"


family("pyposixpath", "pyposixpath", [pyposixpath_line() for _ in range(int(os.environ.get("PYPOSIXPATH_LINES", "8000")))],
       ["python3.13", os.path.join(LIB, "pyposixpath/oracle.py")])


# --- Ruby's Shellwords, against Ruby 2.6 ------------------------------------------------------------------------

RS_PIECES = ["a", "bc", " ", "  ", "\t", "\n", "\x0b", "'", '"', "'q w'", '"q w"', "''", '""', "\\", "\\ ", "\\'", '\\"', "\\\\", "\\\n", '"a\\"b"',
             '"a\\b"', '"\\$x"', "'a\\b'", "é", "😀", "　", "#", "#{", "#$", "+", "$x", "a=b", "--f", "\x00", "\x7f", "\u0085", "​", " ",
             "͸", "\U0010ffff", "", "@", ",", ":", "/", " "]


def rs_text():
    return "".join(rng.choice(RS_PIECES) for _ in range(rng.randrange(0, 8)))


def rubyshellwords_line():
    r = rng.random()
    if r < 0.5:
        return f"sp {rs_text().encode().hex() or '-'}"
    if r < 0.7:
        return f"es {rs_text().encode().hex() or '-'}"
    if r < 0.85:
        args = [rs_text() for _ in range(rng.randrange(0, 4))]
        return f"jo {','.join(a.encode().hex() or '-' for a in args) or '~'}"
    cp = rng.choice([rng.randrange(0x110000), rng.randrange(0x80, 0x3000)])
    if 0xD800 <= cp <= 0xDFFF:
        cp = 0xFFFD
    return f"in {(rs_text() if rng.random() < 0.7 else chr(cp)).encode().hex() or '-'}"


def rubyshellwords_lines():
    return [rubyshellwords_line() for _ in range(int(os.environ.get("RUBYSHELLWORDS_LINES", "8000")))]


family("rubyshellwords", "rubyshellwords", rubyshellwords_lines(), ["ruby", os.path.join(LIB, "rubyshellwords/oracle.rb")])

# The committed table is what the generator writes today.
rb_gen = subprocess.run(["ruby", os.path.join(LIB, "rubyshellwords/gen_tables.rb")], capture_output=True, text=True).stdout
if not open(os.path.join(PKG, "rubyshellwords/src/main.gr"), encoding="utf-8").read().endswith(rb_gen):
    print("packages/rubyshellwords/src/main.gr: the generated table is stale; run scripts/lib/b52/rubyshellwords/gen_tables.rb")
    sys.exit(1)


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
