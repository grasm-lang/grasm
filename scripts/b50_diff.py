"""B50's packages against the references they reproduce.

- JavaScript's numbers in other bases against Node 25 (V8 14.1):
  toString(radix) and parseInt, RangeError messages included.
- encodeURI, encodeURIComponent, decodeURI, decodeURIComponent, escape and
  unescape against Node 25, compared as UTF-16 code units.
- Go 1.25's encoding/json: Valid, Compact, Indent, HTMLEscape, Unmarshal
  into an any (syntax errors with offsets, numbers beyond float64),
  Marshal with and without HTML escaping, and MarshalIndent.
- CPython 3.13's json (the C accelerator): loads, with JSONDecodeError
  messages and positions and the 4300-digit integer limit, and dumps with
  ensure_ascii, allow_nan, sort_keys, indent and separators.
- Go 1.25's encoding/csv: Read record by record (fields, FieldPos, errors,
  InputOffset), ReadAll, and the Writer.
- CPython 3.13's datetime (the C module): timedelta construction,
  arithmetic, str and repr; dates, ordinals and the ISO calendar;
  fromisoformat for date, time and datetime; isoformat with each timespec;
  ctime; fixed-offset timezones; fromtimestamp and timestamp.
- CPython 3.13's fnmatch: translate's text, fnmatchcase and filter.
- Go 1.25's time.Duration: ParseDuration and its errors, String, Round,
  Truncate, Abs, Hours, Minutes and Seconds.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B50_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B50_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B50_LIB") or os.path.join(ROOT, "scripts/lib/b50")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b50")
harness = os.path.join(work, "b50test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b50test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


# --- JavaScript numbers, against Node 25 ----------------------------------------------------------------------

def any_double():
    r = rng.random()
    if r < 0.3:
        return rng.choice([0.0, -0.0, 0.1, 0.5, 1.5, 2.5, -2.5, 1.005, 1e21, 1e20, 999999999999999999999.0, 1e-6, 1e-7, 123.456, 5e-324, 1.7976931348623157e308,
                           float("inf"), -float("inf"), float("nan"), 255.5, 1 / 3, 2 ** 53, 2 ** 53 + 2, 0.000001234, 1e300, 4.35, 0.045])
    if r < 0.55:
        return struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]
    if r < 0.8:
        return round(rng.uniform(-1000, 1000), rng.randrange(0, 6))
    return rng.uniform(-1, 1) * 10 ** rng.randrange(-25, 25)


JS_SPACE = [" ", "\t", "\n", "\u00a0", "\ufeff", "\u2028", "\u3000", "\u180e"]


def numeric_text():
    r = rng.random()
    if r < 0.3:
        return rng.choice(["", " ", "0", "-0", "+0", "00012", "1.", ".5", ".", "+.5e1", "1e", "1e+", "1e5", "-1.5E-3", "Infinity", "-Infinity", "+Infinity", "infinity",
                           "NaN", "0x1F", "0X1f", "0x", "0xg", "-0x1", "0o17", "0O8", "0b101", "0b2", "1_000", "12abc", "abc", "  42  ", "\u00a012\ufeff",
                           "1e400", "-1e400", "4.9e-324", "2.4703282292062327e-324", "9007199254740993", "0.1e-999", "123456789012345678901234567890",
                           "\u180e1", "\u20281", "1\u2029", "٣", "1.5.6", "--1", "+-1"])
    t = rng.choice(["", "-", "+"]) + "".join(rng.choice("0123456789") for _ in range(rng.randrange(1, rng.choice([5, 20, 40]))))
    if rng.random() < 0.4:
        t += "." + "".join(rng.choice("0123456789") for _ in range(rng.randrange(0, 15)))
    if rng.random() < 0.3:
        t += rng.choice("eE") + rng.choice(["", "+", "-"]) + str(rng.randrange(0, 330))
    if rng.random() < 0.2:
        t = rng.choice(JS_SPACE) + t
    if rng.random() < 0.2:
        t += rng.choice(JS_SPACE + ["x", "e", ".", "z"])
    return t


def radix_text(radix):
    alphabet = "0123456789abcdefghijklmnopqrstuvwxyz"[:radix if 2 <= radix <= 36 else 36]
    t = "".join(rng.choice(alphabet + alphabet.upper()) for _ in range(rng.randrange(1, rng.choice([6, 12, 30]))))
    return rng.choice(["", "-", "+", " ", "0x", "0X", "-0x"]) + t + rng.choice(["", "", "z", "!", " "])


def jsradix_line():
    r = rng.random()
    x = "%x" % dbits(any_double())
    if r < 0.4:
        return f"tsr {x} {rng.choice([2, 3, 7, 8, 16, 36, 10, 1, 37, 5, 12])}"
    if r < 0.6:
        # Past 2^53 in a radix that is not a power of two, where V8's fold
        # of 32-bit parts rounds as it goes; leading zeros move the parts.
        radix = rng.choice([3, 5, 6, 7, 11, 12, 20, 36])
        alphabet = "0123456789abcdefghijklmnopqrstuvwxyz"[:radix]
        return f"pi {hx('0' * rng.randrange(0, 4) + ''.join(rng.choice(alphabet) for _ in range(rng.randrange(10, 60))))} {radix}"
    radix = rng.choice([0, 2, 8, 10, 16, 36, 3, 7, 1, 37, 4294967312])
    return f"pi {hx(radix_text(radix) if rng.random() < 0.6 else numeric_text())} {'~' if radix == 0 and rng.random() < 0.5 else radix}"


family("jsradix", "jsradix", [jsradix_line() for _ in range(int(os.environ.get("JSRADIX_LINES", "8000")))], ["node", os.path.join(LIB, "jsradix/oracle.js")])


# --- JavaScript URI functions, against Node 25 -----------------------------------------------------------------

URI_BITS = ["a", "Z", "0", " ", "-_.!~*'()", ";/?:@&=+$,#", "%", "%41", "%2F", "%3b", "%23", "%C3%A9", "%c3%a9", "%E2%82%AC", "%F0%9F%98%80", "%C3", "%C3%41",
            "%E0%80%80", "%ED%A0%80", "%F4%90%80%80", "%80", "%FF", "%zz", "%4", "%u0041", "%u20ac", "%uD800", "%uDC00%", "%u12", "\u00e9", "\u20ac",
            "\U0001F600", "\u0100", "\u00ff", "@*_+-./", "\t", "\n", "\x7f", "\""]


def jsuri_line():
    t = "".join(rng.choice(URI_BITS) for _ in range(rng.randrange(0, 7)))
    return f"{rng.choice(['ec', 'eu', 'dc', 'dc', 'du', 'du', 'es', 'un', 'un'])} {hx(t)}"


family("jsuri", "jsuri", [jsuri_line() for _ in range(int(os.environ.get("JSURI_LINES", "6000")))], ["node", os.path.join(LIB, "jsuri/oracle.js")])


# --- Go's encoding/json, against Go 1.25 -----------------------------------------------------------------------

JSON_NUMBERS = [b"0", b"-0", b"1", b"-1", b"0.1", b"1e400", b"-1e400", b"1e-400", b"1E+2", b"1e-7", b"1e-6", b"1e21", b"1e20", b"123.456e-3", b"5e-324",
                b"1.7976931348623157e308", b"1.7976931348623159e308", b"2.4703282292062328e-324", b"123456789012345678901", b"9007199254740993",
                b"0.000001", b"-0.0", b"100", b"1.5e300", b"12E-1"]
JSON_PIECES = [b"a", b"key", b"Z9", b" ", b'\\"', b"\\\\", b"\\/", b"\\b\\f\\n\\r\\t", b"\\u0041", b"\\u00e9", b"\\u00E9", b"\\ud83d\\ude00", b"\\ud800", b"\\udc00",
               b"\\ud800\\u0041", b"\\ud800\\ud800\\udc00", b"\\u0000", b"\\u001f", b"\\u2028", b"<>&", "é".encode(), "€".encode(), "😀".encode(), b"\x80",
               b"\xff", b"\xed\xa0\x80", b"\xc0\xaf", b"\xf4\x90\x80\x80", b"\xe2\x82", " ".encode(), " ".encode(), b"\x7f", b"'", b"\xef\xbf\xbd"]
JSON_SPACE = [b"", b"", b"", b" ", b"\n", b"\t", b"\r\n  "]


def json_number():
    if rng.random() < 0.4:
        return rng.choice(JSON_NUMBERS)
    t = rng.choice(["", "-"]) + rng.choice(["0", str(rng.randrange(1, 10 ** rng.randrange(1, 20)))])
    if rng.random() < 0.5:
        t += "." + "".join(rng.choice("0123456789") for _ in range(rng.randrange(1, 20)))
    if rng.random() < 0.4:
        t += rng.choice("eE") + rng.choice(["", "+", "-"]) + str(rng.randrange(0, 340))
    return t.encode()


def json_string():
    return b'"' + b"".join(rng.choice(JSON_PIECES) for _ in range(rng.randrange(0, 5))) + b'"'


def json_value(depth=0):
    sp = lambda: rng.choice(JSON_SPACE)
    r = rng.random()
    if depth > 3 or r < 0.45:
        k = rng.random()
        return rng.choice([b"null", b"true", b"false"]) if k < 0.2 else json_number() if k < 0.55 else json_string()
    if r < 0.7:
        return b"[" + sp() + (b"," + sp()).join(json_value(depth + 1) + sp() for _ in range(rng.randrange(0, 4))) + b"]"
    keys = [json_string() for _ in range(rng.randrange(0, 4))]
    if keys and rng.random() < 0.3:
        keys.append(rng.choice(keys))
    return b"{" + sp() + (b"," + sp()).join(k + sp() + b":" + sp() + json_value(depth + 1) + sp() for k in keys) + b"}"


def json_text(deep=False):
    t = rng.choice(JSON_SPACE) + json_value() + rng.choice(JSON_SPACE)
    r = rng.random()
    if r < 0.75:
        return t
    if r < 0.8:
        # The scanner's depth limit, 10000. Not for Indent, whose output at
        # that depth is a hundred megabytes.
        if not deep:
            return t
        return b"[" * 10001 if rng.random() < 0.5 else b"[" * 10000 + b"]" * 10000
    i = rng.randrange(0, len(t) + 1)
    k = rng.random()
    if k < 0.3:
        return t[:i]
    if k < 0.6:
        return t[:i] + t[i + 1:]
    return t[:i] + rng.choice([b",", b"]", b"}", b":", b'"', b"x", b"\x00", b" 1", b"-", b".", b"e", b"tru", b"'", b"\xe9", b"\\"]) + t[i:]


def jhx(b):
    return b.hex() or "-"


INDENTS = [b"", b" ", b"\t", b"  ", b">", b"ab"]


def gojson_line():
    r = rng.random()
    t = json_text(deep=r < 0.25 or 0.45 <= r < 0.85)
    if r < 0.1:
        return f"va {jhx(t)}"
    if r < 0.25:
        return f"co {jhx(t)}"
    if r < 0.4:
        return f"in {jhx(t)} {jhx(rng.choice(INDENTS))} {jhx(rng.choice(INDENTS))}"
    if r < 0.45:
        return f"he {jhx(t)}"
    if r < 0.7:
        return f"un {jhx(t)}"
    if r < 0.85:
        return f"ma {jhx(t)} {rng.choice('01')}"
    if r < 0.9:
        return f"mi {jhx(t)} {jhx(rng.choice(INDENTS))} {jhx(rng.choice(INDENTS))}"
    if r < 0.95:
        return f"nu {dbits(any_double()):x}"
    return f"st {jhx(b''.join(rng.choice(JSON_PIECES) for _ in range(rng.randrange(0, 6))))} {rng.choice('01')}"


gojson_bin = go_tool("gojson", os.path.join(LIB, "gojson/oracle.go"))
family("gojson", "gojson", [gojson_line() for _ in range(int(os.environ.get("GOJSON_LINES", "6000")))], [gojson_bin], chunk=200)


# --- Python's json, against CPython 3.13 -----------------------------------------------------------------------

PY_NUMBERS = ["0", "-0", "1", "-1", "0.0", "-0.0", "1.0", "1e16", "1E+2", "1e-5", "1e400", "-1e400", "1e-400", "123456789012345678901234567890", "0.1", "5e-324",
              "1.7976931348623157e308", "1e22", "12.5e-3", "9007199254740993", "NaN", "Infinity", "-Infinity"]
PY_BAD_NUMBERS = ["1.", "01", "-", "1e", "1e+", "1.e5", ".5", "+1", "-x", "1_0", "0x10", "-Inf", "Infinit", "nan", "NaNx", "-NaN"]
# Pieces of a string literal: the first list is valid, the second is not.
PY_PIECES = ["a", "key", " ", '\\"', "\\\\", "\\/", "\\b\\f\\n\\r\\t", "\\u0041", "\\u00e9", "\\u00E9", "\\ud83d\\ude00", "\\ud800", "\\udc00",
             "\\ud800\\u0041", "\\ud800\\ud800\\udc00", "\\ud800\\udBFF", "\\u0000", "\\u001f", "\\u2028", "é", "€", "😀", " ", "\x7f", "\ud800",
             "\udfff", "'"]
PY_BAD_PIECES = ["\\x", "\\u12", "\\u12G4", "\\ud800\\u12G4", "\\ud800\\", "\\ud800\\u", "\\", "\x00", "\x1f", "\t"]
PY_RAW = ["é", "€", "😀", " ", "\x7f", "\ud800", "\udfff", "\x00", "\x1f", "\t", "a", "\x08", "\\", '"', "\U0010ffff", "￿"]
PY_SPACE = ["", "", "", " ", "\n", "\t", "\r\n  ", "\n\n "]


def py_piece():
    return rng.choice(PY_BAD_PIECES) if rng.random() < 0.04 else rng.choice(PY_PIECES)


def py_number():
    r = rng.random()
    if r < 0.45:
        return rng.choice(PY_NUMBERS)
    if r < 0.5:
        return rng.choice(PY_BAD_NUMBERS)
    if r < 0.53:
        return rng.choice(["", "-"]) + "1" * rng.choice([4300, 4301, 4400])
    t = rng.choice(["", "-"]) + rng.choice(["0", str(rng.randrange(1, 10 ** rng.randrange(1, 30)))])
    if rng.random() < 0.5:
        t += "." + "".join(rng.choice("0123456789") for _ in range(rng.randrange(1, 20)))
    if rng.random() < 0.4:
        t += rng.choice("eE") + rng.choice(["", "+", "-"]) + str(rng.randrange(0, 340))
    return t


def py_value(depth=0):
    sp = lambda: rng.choice(PY_SPACE)
    r = rng.random()
    if depth > 3 or r < 0.45:
        k = rng.random()
        if k < 0.15:
            return rng.choice(["null", "true", "false", "null", "true", "false", "nul", "tru", "fals", "None"])
        if k < 0.5:
            return py_number()
        return '"' + "".join(py_piece() for _ in range(rng.randrange(0, 5))) + '"'
    trailing = "," + sp() if rng.random() < 0.03 else ""
    if r < 0.7:
        return "[" + sp() + ("," + sp()).join(py_value(depth + 1) + sp() for _ in range(rng.randrange(0, 4))) + trailing + "]"
    keys = ['"' + "".join(rng.choice(PY_PIECES + ["b", "A", "b"]) for _ in range(rng.randrange(0, 3))) + '"' for _ in range(rng.randrange(0, 4))]
    if keys and rng.random() < 0.3:
        keys.append(rng.choice(keys))
    return "{" + sp() + ("," + sp()).join(k + sp() + ":" + sp() + py_value(depth + 1) + sp() for k in keys) + trailing + "}"


def py_doc():
    t = rng.choice(PY_SPACE) + py_value() + rng.choice(PY_SPACE)
    r = rng.random()
    if r < 0.85:
        return t
    if r < 0.87:
        return "﻿" + t
    i = rng.randrange(0, len(t) + 1)
    k = rng.random()
    if k < 0.3:
        return t[:i]
    if k < 0.6:
        return t[:i] + t[i + 1:]
    return t[:i] + rng.choice([",", "]", "}", ":", '"', "x", "\x00", " 1", "-", ".", "e", "'", " ", "\x0b", "\\", "\n"]) + t[i:]


def cps(t):
    return ".".join("%x" % ord(c) for c in t) or "-"


def pyjson_line():
    r = rng.random()
    if r < 0.45:
        return f"lo {cps(py_doc())}"
    if r < 0.85:
        indent = rng.choice(["~", "~", "i0", "i2", "i-1", "s" + cps("\t"), "s" + cps("ab"), "s-"])
        return f"du {cps(py_doc())} {rng.choice('01')}{rng.choice('011')}{rng.choice('01')}{rng.choice('00123')} {indent}"
    if r < 0.92:
        return f"df {dbits(any_double()):x} {rng.choice('01')}"
    if r < 0.97:
        return f"ds {cps(''.join(rng.choice(PY_RAW) for _ in range(rng.randrange(0, 6))))} {rng.choice('01')}"
    return f"di {rng.choice(['', '-'])}{rng.choice(['0', '1' * rng.choice([1, 20, 4300, 4301, 5000]), str(rng.randrange(10 ** 40))])}"


family("pyjson", "pyjson", [pyjson_line() for _ in range(int(os.environ.get("PYJSON_LINES", "6000")))], ["python3.13", os.path.join(LIB, "pyjson/oracle.py")],
       chunk=200)


# --- Go's encoding/csv, against Go 1.25 ------------------------------------------------------------------------

CSV_COMMAS = [44, 44, 44, 44, 59, 9, 124, 0xE9, 0x20AC, 32]
CSV_BAD_COMMAS = [34, 10, 13, 0, 0xFFFD, 0xD800]
CSV_COMMENTS = [0, 0, 0, 35, 59, 44, 0xE9]
CSV_PLAIN = [b"a", b"bc", b"12", b"x y", b" lead", b"trail ", b"\xc2\xa0nb", b"\xe3\x80\x80w", b"\xe9", b"\xff", "é".encode(), "€".encode(), b"\\.", b"\t",
             b"#c", b";", b"|", b"\r", b"", b"", b"a\"b", b"\"", b"k\r"]


def csv_field(comma):
    r = rng.random()
    if r < 0.6:
        return rng.choice(CSV_PLAIN)
    inner = b"".join(rng.choice([b"q", b'""', comma, b"\n", b"\r\n", b" ", b"\r", "é".encode(), b"x"]) for _ in range(rng.randrange(0, 4)))
    tail = b"" if rng.random() < 0.85 else rng.choice([b"x", b'"', b" ", b"\r"])
    return b" " * rng.choice([0, 0, 0, 1]) + b'"' + inner + (b'"' if rng.random() < 0.93 else b"") + tail


def csv_data(comma_cp, comment_cp):
    comma = chr(comma_cp).encode("utf-8", "surrogatepass") if comma_cp else b","
    lines = []
    for _ in range(rng.randrange(0, 6)):
        k = rng.random()
        if k < 0.08:
            lines.append(b"")
        elif k < 0.14 and comment_cp:
            lines.append(chr(comment_cp).encode() + b" note")
        else:
            lines.append(comma.join(csv_field(comma) for _ in range(rng.randrange(1, 5))))
    ends = [rng.choice([b"\n", b"\n", b"\r\n", b"\r\n", b"\r"]) for _ in lines]
    data = b"".join(l + e for l, e in zip(lines, ends))
    if data and rng.random() < 0.3:
        data = data[:-1]
    return data


def csv_records():
    recs = []
    for _ in range(rng.randrange(0, 4)):
        fields = [b"".join(rng.choice(CSV_PLAIN + [b",", b";", b"\n", b"\r\n", b"\"", " ".encode()]) for _ in range(rng.randrange(0, 3))) for _ in range(rng.randrange(0, 4))]
        recs.append("r" + ",".join(f.hex() or "~" for f in fields))
    return "/".join(recs) or "-"


def gocsv_line():
    r = rng.random()
    comma = rng.choice(CSV_BAD_COMMAS) if rng.random() < 0.04 else rng.choice(CSV_COMMAS)
    comment = rng.choice(CSV_COMMENTS)
    if r < 0.85:
        data = csv_data(comma, comment)
        opts = f"{comma} {comment} {rng.choice([0, 0, 0, 2, 3, -1])} {rng.choice('001')} {rng.choice('001')}"
        return f"{'rd' if r < 0.7 else 'ra'} {data.hex() or '-'} {opts}"
    return f"wr {csv_records()} {rng.choice([44, 44, 59, 9, 0xE9, 34, 0])} {rng.choice('01')}"


gocsv_bin = go_tool("gocsv", os.path.join(LIB, "gocsv/oracle.go"))
family("gocsv", "gocsv", [gocsv_line() for _ in range(int(os.environ.get("GOCSV_LINES", "6000")))], [gocsv_bin], chunk=200)


# --- Python's datetime, against CPython 3.13 ------------------------------------------------------------------

def td_spec(big=True):
    r = rng.random()
    d = rng.choice([0, 0, 1, -1, 7, -7, 365, -365, 999999999, -999999999, 999999998]) if r < 0.5 else rng.randrange(-999999999 if big else -4000,
                                                                                                               1000000000 if big else 4000)
    return f"{d},{rng.choice([0, 0, 1, 59, 3600, 86399, rng.randrange(86400)])},{rng.choice([0, 0, 1, 500000, 999999, rng.randrange(1000000)])}"


def off_spec(valid=True):
    if rng.random() < 0.15:
        return "0,0,0"
    lim = 86399999999 if valid else 90000000000
    us = rng.choice([3600000000, -3600000000, 19800000000, -12600000000, 1000000, -1, 1, 86399999999, -86399999999, 86400000000, -86400000000,
                     rng.randrange(-lim, lim + 1), rng.randrange(-lim, lim + 1) // 60000000 * 60000000])
    if valid and not -86400000000 < us < 86400000000:
        us = us // 2
    d, rest = divmod(us, 86400000000)
    s, u = divmod(rest, 1000000)
    return f"{d},{s},{u}"


def dt_fields(valid=True):
    y = rng.choice([1, 2, 1970, 2000, 2004, 2021, 9999, rng.randrange(1, 10000)])
    m = rng.randrange(1, 13)
    d = rng.randrange(1, 29) if rng.random() < 0.8 else rng.choice([29, 30, 31])
    if valid:
        d = min(d, [31, 29 if (y % 4 == 0 and (y % 100 != 0 or y % 400 == 0)) else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1])
    else:
        y, m, d = rng.choice([(y, m, d), (0, m, d), (10000, m, d), (y, 0, d), (y, 13, d), (y, m, 0), (y, m, 32), (2021, 2, 29), (2000, 2, 30)])
    t = [rng.randrange(24), rng.randrange(60), rng.randrange(60), rng.choice([0, 0, 1, 500, 999999, rng.randrange(1000000)])]
    if not valid and rng.random() < 0.4:
        k = rng.randrange(4)
        t[k] = rng.choice([-1, [24, 60, 60, 1000000][k]])
    return [y, m, d] + t


def dt_spec():
    f = dt_fields()
    return ",".join(map(str, f)) + "/" + (off_spec() if rng.random() < 0.6 else "~")


def iso_date():
    y, m, d = dt_fields(valid=rng.random() < 0.9)[:3]
    if rng.random() < 0.65:
        return rng.choice([f"{y:04d}-{m:02d}-{d:02d}", f"{y:04d}{m:02d}{d:02d}"])
    w, wd = rng.choice([1, 10, 52, 53, rng.randrange(0, 55)]), rng.choice([1, 7, rng.randrange(0, 9)])
    return rng.choice([f"{y:04d}-W{w:02d}", f"{y:04d}-W{w:02d}-{wd}", f"{y:04d}W{w:02d}", f"{y:04d}W{w:02d}{wd}"])


def iso_time():
    hh, mi, ss = dt_fields(valid=rng.random() < 0.9)[3:6]
    frac = "".join(rng.choice("0123456789") for _ in range(rng.randrange(1, 10)))
    time = rng.choice([f"{hh:02d}", f"{hh:02d}:{mi:02d}", f"{hh:02d}:{mi:02d}:{ss:02d}", f"{hh:02d}{mi:02d}", f"{hh:02d}{mi:02d}{ss:02d}",
                       f"{hh:02d}:{mi:02d}:{ss:02d}.{frac}", f"{hh:02d}:{mi:02d}:{ss:02d},{frac}", f"{hh:02d}{mi:02d}{ss:02d}.{frac}",
                       f"{hh:02d}:{mi:02d}:{ss:02d}:{frac[:3]}", f"{hh:02d}.{frac}"])
    oh, om = rng.choice([0, 1, 5, 12, 23, rng.randrange(0, 26)]), rng.choice([0, 30, 45, 59, 60, 99])
    tz = rng.choice(["", "", "", "Z", f"+{oh:02d}:{om:02d}", f"-{oh:02d}{om:02d}", f"+{oh:02d}", f"-{oh:02d}:{om:02d}:{rng.randrange(60):02d}",
                     f"+{oh:02d}:{om:02d}:{rng.randrange(60):02d}.{frac}", "+00:00", "-00:00", "+00:00:00.5"] + (["z", "+", "Zx", "+1"] if rng.random() < 0.2 else []))
    return time + tz


def mutate(t):
    if rng.random() < 0.08:
        i = rng.randrange(0, len(t) + 1)
        t = rng.choice([t[:i], t[:i] + t[i + 1:], t[:i] + rng.choice(["-", ":", "W", "1", "'", "\\", "\n", "\xa0", "é"]) + t[i:]])
    return t


def iso_text():
    sep = rng.choice(["T", "T", " ", "t", "x", "é", "　", "_", "7"])
    return mutate(iso_date() + sep + iso_time() if rng.random() < 0.9 else iso_date())


def pydatetime_line():
    r = rng.random()
    if r < 0.08:
        vals = [rng.choice([0, 1, -1, rng.randrange(-10 ** rng.randrange(1, 13), 10 ** rng.randrange(1, 13))]) for _ in range(7)]
        if rng.random() < 0.05:
            vals[1] = rng.choice([10 ** 9, 2 ** 31, -2 ** 31 - 1, 10 ** 10])
        return "td " + " ".join(map(str, vals))
    if r < 0.18:
        return f"t2 {rng.choice(['add', 'sub', 'fdt', 'mod', 'div'])} {td_spec()} {td_spec() if rng.random() < 0.9 else '0,0,0'}"
    if r < 0.26:
        return f"t1 {rng.choice(['mul', 'fdi', 'tdi', 'neg', 'abs'])} {td_spec()} {rng.choice([0, 1, -1, 2, 3, -7, 1000, 10 ** 6, rng.randrange(-10 ** 9, 10 ** 9)])}"
    if r < 0.31:
        y, m, d = dt_fields(valid=rng.random() < 0.7)[:3]
        return f"dn {y} {m} {d}"
    if r < 0.34:
        return f"do {rng.choice([0, 1, -1, 3652059, 3652060, rng.randrange(1, 3652060)])}"
    if r < 0.38:
        return f"dc {rng.choice([0, 1, 2004, 2009, 2015, 2020, 9999, 10000, rng.randrange(1, 10000)])} {rng.randrange(-1, 55)} {rng.randrange(-1, 9)}"
    if r < 0.48:
        return f"di {cps(mutate(iso_date()) if rng.random() < 0.85 else iso_text())}"
    if r < 0.56:
        return f"ti {cps(mutate(rng.choice(['', '', 'T']) + iso_time()))}"
    if r < 0.72:
        return f"xi {cps(iso_text())}"
    if r < 0.76:
        return f"da {','.join(map(str, dt_fields()[:3]))} {td_spec(big=rng.random() < 0.2)} {rng.choice('+-')}"
    if r < 0.78:
        return f"dd {','.join(map(str, dt_fields()[:3]))} {','.join(map(str, dt_fields()[:3]))}"
    if r < 0.83:
        return f"xa {dt_spec()} {td_spec(big=rng.random() < 0.2)} {rng.choice('+-')}"
    if r < 0.87:
        a, b = dt_spec(), dt_spec()
        if rng.random() < 0.7:
            b = b.split("/")[0] + "/" + (a.split("/")[1] if a.split("/")[1] == "~" else off_spec())
        return f"xd {a} {b}"
    if r < 0.93:
        ts = rng.choice([0.0, -0.0, 1e-7, 5e-7, 1.5e-6, 2.5e-6, -5e-7, 1e16, -1e16, 1e12, -62135596800.0, -62135596801.0, 253402300799.999999,
                         253402300800.0, float("inf"), float("nan"), rng.uniform(-62135596800, 253402300800),
                         rng.randrange(-62135596800, 253402300800) + rng.choice([0.5e-6, 0.25, 0.0000015, 0.9999995])])
        return f"xt {dbits(ts):x} {off_spec()}"
    if r < 0.95:
        return f"tz {off_spec(valid=False)}"
    if r < 0.97:
        f = dt_fields(valid=rng.random() < 0.8)
        return f"tn {' '.join(map(str, f[3:]))} {off_spec() if rng.random() < 0.5 else '~'}"
    f = dt_fields(valid=rng.random() < 0.85)
    spec = rng.choice(["auto", "hours", "minutes", "seconds", "milliseconds", "microseconds", "bogus"])
    return f"xn {' '.join(map(str, f))} {off_spec() if rng.random() < 0.5 else '~'} {rng.choice([84, 32, 116, 233, 12288])} {spec}"


family("pydatetime", "pydatetime", [pydatetime_line() for _ in range(int(os.environ.get("PYDATETIME_LINES", "6000")))],
       ["python3.13", os.path.join(LIB, "pydatetime/oracle.py")], chunk=200)


# --- Python's fnmatch, against CPython 3.13 -------------------------------------------------------------------

FN_PIECES = ["a", "b", "c", ".", "/", "-", "\\", "x", "é", "😀", " ", "\n", "(", ")", "{", "}", "+", "^", "$", "|", "&", "~", "#", "\t", "]", "!",
             "*", "*", "**", "?", "?", "[abc]", "[!a-c]", "[a-]", "[]a]", "[!]a]", "[!]", "[]", "[z-a]", "[a-c-e]", "[a--c]", "[\\]", "[^x]", "[[]",
             "[&&]", "[~|]", "[", "[!", "[a-z]", "[!.]", "[-a]", "[!-]", "[--]", "[a-\\]", "[\\-z]", "[é-😀]", "[]-a]", "[!a-c-b]", "[a-ce-g]",
             "[.-/]", "[*]", "[?]", "[!*]", "[!z-a]", "[!d-a]", "[c-a]", "[!c-a-b]"]
FN_CHARS = ["a", "b", "c", ".", "/", "-", "\\", "x", "é", "😀", " ", "\n", "[", "]", "!", "^", "&", "~", "|", "z", "*", "?", "(", "d", "e", "f"]


def fn_pattern():
    return "".join(rng.choice(FN_PIECES) for _ in range(rng.randrange(0, 7)))


def fn_name(pat):
    # Often an instance of the pattern, so that matches are common.
    if rng.random() < 0.5:
        out = []
        i = 0
        while i < len(pat):
            c = pat[i]
            if c == "*":
                out.append("".join(rng.choice(FN_CHARS) for _ in range(rng.randrange(0, 3))))
            elif c == "?":
                out.append(rng.choice(FN_CHARS))
            elif c == "[":
                j = pat.find("]", i + 2)
                inside = pat[i + 1:j] if j > 0 else ""
                out.append(rng.choice(inside) if inside and rng.random() < 0.6 else rng.choice(FN_CHARS))
                i = j if j > 0 and rng.random() < 0.8 else i
            else:
                out.append(c)
            i += 1
        return "".join(out)
    return "".join(rng.choice(FN_CHARS) for _ in range(rng.randrange(0, 7)))


def pyfnmatch_line():
    r = rng.random()
    p = fn_pattern()
    if r < 0.3:
        return f"tr {cps(p)}"
    if r < 0.85:
        return f"mc {cps(fn_name(p))} {cps(p)}"
    return f"fi {'/'.join(cps(fn_name(p)) for _ in range(rng.randrange(1, 5)))} {cps(p)}"


family("pyfnmatch", "pyfnmatch", [pyfnmatch_line() for _ in range(int(os.environ.get("PYFNMATCH_LINES", "8000")))],
       ["python3.13", os.path.join(LIB, "pyfnmatch/oracle.py")], chunk=500)


# --- Go's time.Duration, against Go 1.25 ----------------------------------------------------------------------

DUR_UNITS = ["ns", "us", "µs", "μs", "ms", "s", "m", "h", "", "x", "S", "hr", "mss", "µ", "\udcb5s", "\udcffx", "\t", "\x01s", "\"", "\\", "\x7f"]
I64_MIN, I64_MAX = -(1 << 63), (1 << 63) - 1


def dur_number():
    r = rng.random()
    if r < 0.4:
        return str(rng.randrange(0, 1000))
    if r < 0.6:
        return str(rng.randrange(0, 10 ** rng.randrange(1, 21)))
    if r < 0.85:
        return rng.choice(["", "0", "1", "12"]) + "." + "".join(rng.choice("0123456789") for _ in range(rng.randrange(0, 25)))
    return rng.choice(["9223372036854775807", "9223372036854775808", "9223372036854775809", "922337203685477580", "2562047", "2562048",
                       "153722867", "153722868", "9223372036", "9223372037", "18446744073709551616", "0.9999999999999999999999", ".", ""])


def dur_text():
    r = rng.random()
    if r < 0.05:
        return rng.choice(["", "0", "-0", "+0", "-", "+", "00", "0s", "-9223372036854775808ns", "9223372036854775807ns", "9223372036854775808ns",
                           "-9223372036854775809ns", "2562047h47m16.854775807s", "2562047h47m16.854775808s", "1h1h", "1.5h-1m", "1 s", "1e3s",
                           "9223372036854775808ns9223372036854775808ns"])
    parts = [dur_number() + rng.choice(DUR_UNITS[:8] if rng.random() < 0.9 else DUR_UNITS) for _ in range(rng.randrange(1, 4))]
    return rng.choice(["", "", "-", "+"]) + "".join(parts)


def dur_int():
    r = rng.random()
    if r < 0.3:
        return rng.choice([0, 1, -1, 999, 1000, 999999, 1000000, 999999999, 1000000000, 60000000000, 3600000000000, I64_MIN, I64_MAX, I64_MIN + 1,
                           -999, -1000000000, 1500, 1500000])
    if r < 0.7:
        return rng.randrange(-10 ** rng.randrange(1, 19), 10 ** rng.randrange(1, 19))
    return rng.randrange(I64_MIN, I64_MAX + 1)


def goduration_line():
    r = rng.random()
    if r < 0.55:
        return f"pd {dur_text().encode('utf-8', 'surrogateescape').hex() or '-'}"
    if r < 0.8:
        return f"ds {dur_int()}"
    m = rng.choice([0, -1, 1, 2, 10, 1000, 1000000, 1000000000, 60000000000, 3600000000000, I64_MAX, rng.randrange(1, 10 ** 12)])
    return f"rt {dur_int()} {m}"


goduration_bin = go_tool("goduration", os.path.join(LIB, "goduration/oracle.go"))
family("goduration", "goduration", [goduration_line() for _ in range(int(os.environ.get("GODURATION_LINES", "8000")))], [goduration_bin], chunk=500)


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
