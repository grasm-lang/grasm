"""B54's packages against the references they reproduce.

- SQLite 3.51.1's date and time functions (date, time, datetime,
  julianday, unixepoch, strftime, timediff) with their modifiers, through
  CPython 3.13's sqlite3 module.
- SQLite 3.51.1's printf(): every conversion and flag, `*` arguments,
  and arguments of every SQL type.
- Lua 5.5's patterns: string.find (plain and not), match, gmatch and gsub
  with a string or a table, errors included.
- Go 1.25's flag package: FlagSet.Parse, the values and Args after it,
  and PrintDefaults.
- Node 25's util.parseArgs: tokens, values, positionals and the errors,
  in strict and loose mode.
- Perl 5.34's version objects (version 0.9928): parse and declare from
  strings, stringify, numify, normal, is_alpha, is_qv and <=>.
- Rust 1.99's f64 Display, Debug, {:.N}, {:e}, {:E} and {:.Ne}, and
  str::parse for f64 and the integer types, errors included.
- Java 17's java.time.LocalDate: parse, of, ofEpochDay, toString, the
  plus and with methods, until, and the calendar queries, exceptions
  included.
- Data::Dumper 2.179 (XS, Perl 5.34) over graphs of SVs: indent styles,
  Useqq, Quotekeys, Purity, Deepcopy, bless, names, shared and cyclic
  references, Maxrecurse.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B54_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B54_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B54_LIB") or os.path.join(ROOT, "scripts/lib/b54")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b54")
harness = os.path.join(work, "b54test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b54test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


# The SQLite these families reproduce is 3.51.1, reached through Python's
# sqlite3 module, which loads whatever libsqlite3 Homebrew links today.
# GRASM_SQLITE_ORACLE names the 3.51.1 library directory (allgates.sh sets it);
# DYLD_LIBRARY_PATH is set here, on the oracle process, because macOS strips it
# from anything started through /usr/bin/env.
ORACLE_ENV = dict(os.environ)
if os.path.isdir(os.environ.get("GRASM_SQLITE_ORACLE", "")):
    ORACLE_ENV["DYLD_LIBRARY_PATH"] = os.environ["GRASM_SQLITE_ORACLE"]


def family(name, op, lines, oracle_argv, chunk=500):
    want = subprocess.run(oracle_argv, input="\n".join(lines) + "\n", capture_output=True, text=True, env=ORACLE_ENV).stdout.splitlines()
    if len(want) != len(lines):
        print(f"{name}: the oracle answered {len(want)} of {len(lines)}"); sys.exit(1)
    for k in range(0, len(lines), chunk):
        cases.append((name, op, case_file("\n".join(lines[k:k + chunk]) + "\n"), want[k:k + chunk]))


# --- SQLite's date and time functions, against SQLite 3.51.1 ----------------------------------------------------

def sd_date_text():
    good = rng.random() < 0.85
    y = f"{rng.randrange(0, 10000):04d}" if good and rng.random() < 0.7 else rng.choice(["2024", "2000", "1970", "0000", "9999", "-4713", "-0001", "1900",
                                                                                          "2100"] + ([] if good else ["12345", "199"]))
    m = f"{rng.randrange(1, 13):02d}" if good else rng.choice(["00", "13", "1"])
    d = f"{rng.randrange(1, 32):02d}" if good or rng.random() < 0.5 else rng.choice(["00", "32"])
    t = y + "-" + m + "-" + d
    if rng.random() < 0.7:
        t += rng.choice([" ", "T", " ", "T", "  ", "t", " T "]) + sd_time_text()
    return t + (rng.choice(["", "", "", " "]) if good else rng.choice(["x", "Z", " +"]))


def sd_time_text():
    good = rng.random() < 0.9
    h = f"{rng.randrange(0, 24):02d}" if good else rng.choice(["24", "25", "1"])
    m = f"{rng.randrange(0, 60):02d}" if good else rng.choice(["59", "60"])
    t = h + ":" + m
    if rng.random() < 0.6:
        t += ":" + (f"{rng.randrange(0, 60):02d}" if good else rng.choice(["59", "60", "5"]))
        if rng.random() < 0.5:
            t += "." + ("".join(rng.choice("0123456789") for _ in range(rng.randrange(1, 8))) if rng.random() < 0.7 else
                        rng.choice(["0005", "9995", "1235", "9999", "99999", "0015", "5", "4995", "12345678901234567890"]))
    if rng.random() < 0.25:
        t += rng.choice(["Z", "z", " Z", "+04:00", "-05:30", "+00:00", "-00:00", "+14:00", " +01:00"] + ([] if good else ["+15:00", "+04:60", "+0100", "+1:00"]))
    return t


def sd_edge_number():
    # Where a product lands half a millisecond from a whole one, so a fused
    # multiply-add and a separate one can round to different integers.
    import math
    k = rng.randrange(1, 464269060799999)
    x = (k - 0.5) / 86400000.0
    for _ in range(rng.randrange(0, 3)):
        x = math.nextafter(x, rng.choice([0.0, 1e300]))
    return x


def sd_midpoint_text():
    # A decimal exactly half way between two adjacent doubles, which only a
    # careful parser reads the way SQLite's double-double one does.
    import decimal
    import math
    x = rng.uniform(0, 5373484.5) if rng.random() < 0.7 else rng.uniform(-2.1e11, 2.6e11)
    y = math.nextafter(x, 1e300)
    decimal.getcontext().prec = 60
    m = (decimal.Decimal(x) + decimal.Decimal(y)) / 2
    return format(m, "f") if rng.random() < 0.7 else format(m.normalize(), "e")


def sd_number():
    r = rng.random()
    if r < 0.15:
        return sd_edge_number()
    if r < 0.4:
        return rng.uniform(0, 5373484.5)
    if r < 0.7:
        return rng.uniform(-2.1e11, 2.6e11)
    return rng.choice([0.0, -0.0, 0.5, 2440587.5, 2451544.5, 5373484.4999, 5373484.5, -1.0, 1e300, 1.5e9, 1e10, 253402300799.0, 253402300800.0,
                       -210866760000.0, -210866760001.0, float(rng.randrange(0, 2 ** 31))])


def sd_number_text():
    if rng.random() < 0.4:
        return sd_midpoint_text() if rng.random() < 0.6 else repr(sd_edge_number())
    return rng.choice(["2460000.5", "1e3", "-5", "  123  ", "1.5e9", ".5", "5.", "12abc", "2440587.5", "1e", "+7", "0", "1700000000", "2460000.25000000001",
                       "1234567890123456789", "3.14159265358979323846", "1E-3", "0x10", ""])


def sd_value():
    r = rng.random()
    if r < 0.55:
        return "t" + (sd_date_text().encode().hex() or "-")
    if r < 0.67:
        return "t" + sd_time_text().encode().hex()
    if r < 0.75:
        return "t" + (sd_number_text().encode().hex() or "-")
    if r < 0.82:
        return "n" + struct.pack(">d", rng.uniform(2378496.5, 2524593.5)).hex()
    if r < 0.87:
        return "i" + str(rng.choice([0, 1, -1, 2460000, 1704067200, 5373484, 5373485, -210866760000, 2 ** 53 + 1, 2 ** 63 - 1, -2 ** 63,
                                     rng.randrange(-10 ** 7, 10 ** 7)]))
    if r < 0.9:
        return "z"
    if r < 0.94:
        t = sd_date_text().encode()
        return "b" + ((t + rng.choice([b"", b"\x00", b"\x00junk", b"\xff"])).hex() or "-")
    if r < 0.96:
        head = sd_date_text() if rng.random() < 0.5 else sd_number_text()
        return "t" + (head + "\x00" + rng.choice(["", "x", " 12:00", "5", "e3"])).encode().hex()
    return "n" + struct.pack(">d", sd_number()).hex()


SD_UNITS = ["day", "days", "DAYS", "hour", "hours", "minute", "minutes", "second", "seconds", "month", "months", "year", "years", "Month"]
SD_BAD_UNITS = ["secs", "week", "fortnight", "s"]


def sd_modifier():
    good = rng.random() < 0.85
    r = rng.random()
    if r < 0.45:
        n = rng.choice([str(rng.randrange(-400, 400)), f"{rng.uniform(-50, 50):.{rng.randrange(0, 4)}f}", "1", "-1", "0.5", "1e3", "+12", "+1.25", "13"] +
                       ([] if good else ["1e7", "-1e9", "5373485", "14713", "176546"]))
        if rng.random() < 0.6 and not n.startswith("-") and not n.startswith("+"):
            n = "+" + n
        return n + rng.choice([" ", "  ", " ", ""]) + rng.choice(SD_UNITS if good else SD_BAD_UNITS)
    if r < 0.58:
        sign = rng.choice(["+", "-"] if good else ["+", "-", ""])
        y = rng.choice([f"{rng.randrange(0, 50):04d}", "0001", f"{rng.randrange(0, 20):05d}"])
        t = f"{sign}{y}-{rng.randrange(0, 12 if good else 13):02d}-{rng.randrange(0, 31 if good else 32):02d}"
        if rng.random() < 0.4:
            t += " " + f"{rng.randrange(0, 24):02d}:{rng.randrange(0, 60):02d}" + rng.choice(["", f":{rng.randrange(0, 60):02d}", ":05.25"])
        return t
    if r < 0.7:
        return rng.choice(["+", "-", ""]) + sd_time_text()
    if good:
        return rng.choice(["start of day", "start of month", "start of year", "START OF MONTH", "weekday 0", "weekday 1", "weekday 3", "weekday 6",
                           "WEEKDAY 5", "weekday 2.0", "unixepoch", "julianday", "auto", "floor", "ceiling", "subsec", "subsecond", "UNIXEPOCH"])
    return rng.choice(["start of week", "start of", "weekday 7", "weekday 1.5", "weekday -1", "bogus", "", "+", "-"])


def sd_mods():
    if rng.random() < 0.06:
        # julianday, unixepoch and auto are only allowed first.
        pair = [rng.choice(["+1 day", "subsec", "+0 seconds", "start of day"]), rng.choice(["julianday", "unixepoch", "auto"])]
        return " ".join("t" + m.encode().hex() for m in pair)
    if rng.random() < 0.15:
        pair = [rng.choice(["+1 month", "-1 month", "+13 months", "+1 year", "-3 months", "+11 months", "+0001-01-00", "-0000-01-00"]),
                rng.choice(["floor", "ceiling", "floor", "start of day"])]
        return " ".join("t" + m.encode().hex() for m in pair)
    if rng.random() < 0.1:
        return "t" + (rng.choice(["+", "-"]) + repr(abs(sd_edge_number()) / rng.choice([1.0, 24.0, 1440.0])) + " days").encode().hex()
    return " ".join("t" + (sd_modifier().encode().hex() or "-") for _ in range(rng.choice([0, 0, 1, 1, 2, 3])))


SD_SPECS = list("deFfGgHIjJklmMpPRsSTuUVwWY%") + ["Q", "y", ""]


def sqlitedatetime_line():
    r = rng.random()
    if r < 0.03:
        # Under a millisecond, where %f rounds the first digit by itself.
        frac = "." + rng.choice(["0004", "0005", "0006", "00049", "00051", "0009", "0001"])
        t = f"{rng.randrange(0, 24):02d}:{rng.randrange(0, 60):02d}:{rng.randrange(0, 60):02d}{frac}"
        return f"sf t{'%f %S'.encode().hex()} t{t.encode().hex()}"
    if r < 0.12:
        return f"jd {sd_value()} {sd_mods()}".rstrip()
    if r < 0.22:
        return f"ue {sd_value()} {sd_mods()}".rstrip()
    if r < 0.32:
        return f"date {sd_value()} {sd_mods()}".rstrip()
    if r < 0.4:
        return f"time {sd_value()} {sd_mods()}".rstrip()
    if r < 0.6:
        return f"dt {sd_value()} {sd_mods()}".rstrip()
    if r < 0.9:
        specs = SD_SPECS if rng.random() < 0.1 else SD_SPECS[:-3]
        fmt = "".join(("%" + rng.choice(specs)) if rng.random() < 0.75 else rng.choice(["-", " ", ":", "x", "é"]) for _ in range(rng.randrange(1, 8)))
        if rng.random() < 0.05:
            fmt += "%"
        return f"sf t{fmt.encode().hex()} {sd_value()} {sd_mods()}".rstrip()
    return f"td {sd_value()} {sd_value()}"


family("sqlitedatetime", "sqlitedatetime", [sqlitedatetime_line() for _ in range(int(os.environ.get("SQLITEDATETIME_LINES", "10000")))],
       ["python3.13", os.path.join(LIB, "sqlitedatetime/oracle.py")])


# --- Lua's patterns, against Lua 5.5 ---------------------------------------------------------------------------

LP_CLASSES = ["a", "b", "x", " ", ".", "%a", "%A", "%d", "%D", "%s", "%S", "%w", "%W", "%p", "%P", "%l", "%u", "%c", "%x", "%g", "%z", "%%", "%.",
              "%-", "%]", "[abc]", "[^abc]", "[a-z]", "[%d_]", "[^%s]", "[]]", "[^]]", "[a-]", "[-a]", "[%a-]", "%é", "é", "\xff", "[\x80-\xff]", "%q"]
LP_QUANT = ["", "", "", "*", "+", "-", "?"]
LP_BAD = ["%", "[a", "[^", "%b", "%ba", "%f", "%fa", "(", ")", "%1", "%2", "%0", "(()", "(%1)"]


def lp_pattern():
    out = []
    if rng.random() < 0.15:
        out.append("^")
    for _ in range(rng.randrange(0, 6)):
        r = rng.random()
        if r < 0.55:
            out.append(rng.choice(LP_CLASSES) + rng.choice(LP_QUANT))
        elif r < 0.7:
            inner = "".join(rng.choice(LP_CLASSES) + rng.choice(LP_QUANT) for _ in range(rng.randrange(0, 3)))
            out.append("(" + inner + ")")
        elif r < 0.76:
            out.append("()")
        elif r < 0.84:
            out.append(rng.choice(["%b()", "%b[]", "%bab", "%b\"\"", "%f[%w]", "%f[%W]", "%f[%a%d]", "%f[^%s]", "%f[%z]"]))
        elif r < 0.9:
            out.append("%" + str(rng.randrange(1, 4)))
        else:
            out.append(rng.choice(LP_BAD))
    if rng.random() < 0.15:
        out.append("$")
    return "".join(out)


def lp_subject():
    if rng.random() < 0.05:
        return "a" * rng.choice([150, 198, 199, 200, 201, 250])
    alphabet = rng.choice(["abc", "ab xy", "a1 b2", "(a(b)c)", "[x]", "hello world", "key=val; k2=v2", "é\xff\x00 ", "%.-*", "aaaa"])
    return "".join(rng.choice(alphabet) for _ in range(rng.randrange(0, 16)))


def lp_hex(t):
    return t.encode("latin-1").hex() or "-"


def luapattern_line():
    if rng.random() < 0.01:
        # Lua gives up 200 levels deep: 199 optional items match, 200 do not.
        subject, pattern = lp_hex("a" * rng.choice([200, 250])), lp_hex("a?" * rng.choice([199, 200]))
        return f"m {subject} {pattern} 1" if rng.random() < 0.5 else f"f {subject} {pattern} 1 0"
    s = lp_subject()
    p = lp_pattern()
    if len(s) >= 150 and rng.random() < 0.7:
        p = rng.choice(["a?" * rng.choice([150, 199, 200, 201]), "(a)-" * 3, "a-b", "(.-)%1"])
    init = rng.choice([1, 1, 1, 0, 2, 3, -1, -3, -100, 100])
    r = rng.random()
    if r < 0.3:
        return f"f {lp_hex(s)} {lp_hex(p)} {init} {rng.choice('0001')}"
    if r < 0.5:
        return f"m {lp_hex(s)} {lp_hex(p)} {init}"
    if r < 0.7:
        return f"g {lp_hex(s)} {lp_hex(p)} {init}"
    n = rng.choice(["~", "~", "~", "0", "1", "2", "-1"])
    if r < 0.9:
        repl = rng.choice(["<%0>", "%1", "%2", "[%1|%2]", "x", "", "%%", "%", "%a", "%9", "-%0-"])
        return f"r {lp_hex(s)} {lp_hex(p)} {lp_hex(repl)} {n}"
    keys = rng.sample(["a", "b", "ab", "x", "hello", "1", "key", "k2", " "], 3)
    table = ",".join(lp_hex(k) + "=" + lp_hex(k.upper() + "!") for k in keys)
    return f"t {lp_hex(s)} {lp_hex(p)} {table} {n}"


family("luapattern", "luapattern", [luapattern_line() for _ in range(int(os.environ.get("LUAPATTERN_LINES", "10000")))],
       ["lua", os.path.join(LIB, "luapattern/oracle.lua")])


# --- Go's flag package, against Go 1.25 -------------------------------------------------------------------------

# Per kind: values its Set accepts, then values it refuses.
GF_KINDS = {"b": (["true", "false", "1", "0", "t", "F", "TRUE"], ["yes", "", "2"]),
            "i": (["5", "-3", "0x1F", "0o17", "017", "0b101", "1_000", "-9223372036854775808"], ["9223372036854775808", "x", "", "1e3"]),
            "I": (["7", "-9223372036854775808", "0X_FF"], ["1e3", "0x"]),
            "u": (["5", "0x10", "18446744073709551615", "0b1"], ["-1", "18446744073709551616"]),
            "U": (["42", "0b11", "1_2"], ["1__2", "+1"]),
            "s": (["", "hello", "two words", "q\"uote", "é", "tab\t", "\x7f"], []),
            "f": (["1.5", "-0", "0x1p-2", "inf", "NaN", "1_0.5", "1e-400", "+Inf", "1e308"], ["1e400", "x", "0x1p", "1__0"]),
            "d": (["1s", "1h30m", "-5ms", "0", "1.5h", "100ns", "-0", ".5s"], ["2x", "100", "1d", ""])}
GF_NAMES = ["v", "n", "name", "x", "verbose", "count", "é", "a-b", "help2", "h", "help", "dur", "rate", "out"]
GF_USAGE = ["", "print more", "set the `size` of things", "a `name`", "first line\nsecond line", "one ` quote", "the count"]


def gf_defs():
    names = rng.sample(GF_NAMES, rng.randrange(1, 6))
    names = [n for n in names if not (n in ("h", "help") and rng.random() < 0.7)] or ["v"]
    specs, kinds = [], {}
    for n in names:
        k = rng.choice(list(GF_KINDS))
        kinds[n] = k
        good, bad = GF_KINDS[k]
        d = "~" if rng.random() < 0.4 else ((rng.choice(good) if rng.random() < 0.97 or not bad else rng.choice(bad)).encode().hex() or "-")
        specs.append(f"{k}:{n.encode().hex()}:{d}:{rng.choice(GF_USAGE).encode().hex() or '-'}")
    return ",".join(specs), kinds


def gf_value(k):
    good, bad = GF_KINDS[k]
    return rng.choice(good) if rng.random() < 0.85 or not bad else rng.choice(bad)


def gf_arg(kinds):
    r = rng.random()
    names = list(kinds)
    n = rng.choice(names) if rng.random() < 0.88 else rng.choice(["undefined", "help", "h"])
    k = kinds.get(n, "s")
    dash = rng.choice(["-", "-", "--"])
    if r < 0.35:
        return [dash + n] + ([] if k == "b" or rng.random() < 0.1 else [gf_value(k)])
    if r < 0.75:
        return [dash + n + "=" + gf_value(k)]
    if r < 0.92:
        return [rng.choice(["arg", "file.txt", "-", "x y"])]
    return [rng.choice(["--", "---x", "-=x", "-a=b=c", "--="])]


def goflag_line():
    defs, kinds = gf_defs()
    if rng.random() < 0.15:
        return f"d {defs}"
    args = [a for _ in range(rng.randrange(0, 6)) for a in gf_arg(kinds)]
    return f"p {defs} " + (",".join(a.encode().hex() or "-" for a in args) if args else "~")


family("goflag", "goflag", [goflag_line() for _ in range(int(os.environ.get("GOFLAG_LINES", "8000")))],
       [go_tool("goflag", os.path.join(LIB, "goflag/oracle.go"))])


# --- Node's util.parseArgs, against Node 25 ---------------------------------------------------------------------

PA_NAMES = ["all", "verbose", "o", "out", "x", "no-x", "1", "2", "10", "01", "a", "b", "file", "é", "name", "c"]
PA_SHORTS = ["a", "b", "o", "v", "x", "1", "c", "é", "😀", "ab", ""]


def pa_options():
    opts, defined = [], []
    for n in rng.sample(PA_NAMES, rng.randrange(0, 6)):
        t = rng.choice("sb")
        short = rng.choice(PA_SHORTS) if rng.random() < 0.6 else None
        if short in ("ab", "", "😀") and rng.random() < 0.95:
            short = "a"
        multiple = rng.random() < 0.25
        d = "~"
        if rng.random() < 0.3:
            one = (lambda: rng.choice(["d", "", "x y", "é"]).encode().hex() or "-") if t == "s" else (lambda: rng.choice("01"))
            d = "+".join(one() for _ in range(rng.randrange(0, 3))) if multiple else one()
            if multiple and d == "":
                d = "~"
        defined.append((n, t, short))
        opts.append(f"{n.encode().hex()}:{t}:{(short.encode().hex() or '-') if short is not None else '~'}:{'1' if multiple else '0'}:{d}")
    return ",".join(opts) or "~", defined


PA_ARGS = ["-a", "-b", "-o", "-v", "-x", "-ab", "-abc", "-ob", "-ofile", "-ba", "-1", "-é", "-😀", "-😀x", "--all", "--verbose", "--out", "--out=f",
           "--out=", "--no-x", "--x", "--1", "--2=v", "--name=a=b", "--=x", "---", "--", "-", "pos", "file.txt", "-c", "--c", "--no-c", "--é=é",
           "--__proto__", "--10", "--01=x", "-av", "-xo", "-oo"]


def pa_arg(defined):
    if defined and rng.random() < 0.7:
        n, t, short = rng.choice(defined)
        r = rng.random()
        if short and r < 0.3:
            return ["-" + short + (rng.choice(["", "val", "-v"]) if t == "s" and rng.random() < 0.4 else "")]
        if r < 0.45 and short:
            others = "".join(sh for (_n, _t, sh) in defined if sh and len(sh) == 1)
            return ["-" + "".join(rng.choice(others) for _ in range(rng.randrange(2, 4)))] if others else ["-" + short]
        if r < 0.75:
            return ["--" + (("no-" + n) if t == "b" and rng.random() < 0.2 else n)] + ([rng.choice(["val", "-v", "x y"])] if t == "s" and rng.random() < 0.7 else [])
        return ["--" + n + "=" + rng.choice(["v", "", "-v", "a=b"])]
    return [rng.choice(PA_ARGS) if rng.random() < 0.85 else rng.choice(["val", "-val", "--val", "-"])]


def nodeparseargs_line():
    opts, defined = pa_options()
    args = [x for _ in range(rng.randrange(0, 6)) for x in pa_arg(defined)]
    a = ",".join(x.encode().hex() or "-" for x in args) or "~"
    return f"{a} {opts} {rng.choice('1110')} {rng.choice(['~', '~', '1', '0'])} {rng.choice('01')}"


family("nodeparseargs", "nodeparseargs", [nodeparseargs_line() for _ in range(int(os.environ.get("NODEPARSEARGS_LINES", "8000")))],
       ["node", os.path.join(LIB, "nodeparseargs/oracle.js")])


# --- Perl's version objects, against Perl 5.34 (version 0.9928) --------------------------------------------------

PV_PIECES = ["1", "0", "2", "10", "001", "999", "1000", "12345", "007", "1", "2", "3", "02", "100"]
# Past 2^31 - 1 a version is v.Inf; how the digits get there decides when:
# a run of zeros saturates the multiplier before any digit is large.
PV_BIG = ["99999999999", "2147483647", "2147483648", "10000000000", "00000000000", "000000000001", "0000000000001", "1000000000"]


def pv_text():
    r = rng.random()
    if r < 0.12:
        return rng.choice(["", "undef", " ", "v", "v1", "1.", "1..2", "1.2.", "1.2.3.", "-1", ".5", "1_2", "1._2", "1.2_", "1.2_3_4", "1_2.3", "v1.2_3",
                           "1.2.3_4", "1e3", "0x10", "1.2a", "1.2 ", " 1.2", "1.2;x", "1.2{", "1.2 3", "v1.2.3.4.5", "1,2", "1.2,3", "١.٢"])
    parts = [rng.choice(PV_PIECES) if rng.random() < 0.9 else rng.choice(PV_BIG) for _ in range(rng.choice([1, 2, 2, 3, 3, 4]))]
    t = ("v" if rng.random() < 0.25 else "") + ".".join(parts)
    if rng.random() < 0.2:
        i = rng.randrange(1, len(t) + 1)
        t = t[:i] + "_" + t[i:]
    if rng.random() < 0.1:
        t = rng.choice([" ", "  "]) + t + rng.choice(["", " ", ";", "{x", "}"])
    return t


def perlversion_line():
    if rng.random() < 0.3:
        return f"c {pv_text().encode().hex() or '-'} {rng.choice('pd')} {pv_text().encode().hex() or '-'} {rng.choice('pd')}"
    return f"{rng.choice('ppd')} {pv_text().encode().hex() or '-'}"


family("perlversion", "perlversion", [perlversion_line() for _ in range(int(os.environ.get("PERLVERSION_LINES", "8000")))],
       ["perl", os.path.join(LIB, "perlversion/oracle.pl")])


# --- Rust's numbers as text, against Rust 1.99 ------------------------------------------------------------------

def rn_double():
    import math
    r = rng.random()
    if r < 0.25:
        return rng.choice([0.0, -0.0, 1.0, 0.1, 0.25, 2.5, 1.5, 0.5, 1e15, 1e16, 9.999999999999998e15, 1e21, 1e-4, 0.00009999999999999999, 1e-5, 1e-7, 5e-324,
                           1.7976931348623157e308, float("inf"), float("-inf"), float("nan"), 123456789012345680.0, 0.3, 2.0 ** 53, -1.5e-5, 100.0])
    if r < 0.5:
        return rng.choice([1, -1]) * rng.uniform(0, 10) * 10.0 ** rng.randrange(-30, 30)
    if r < 0.7:
        return float(rng.randrange(-10 ** 6, 10 ** 6)) / rng.choice([1, 2, 4, 8, 10, 100, 1000])
    bits = rng.getrandbits(64)
    return struct.unpack("<d", struct.pack("<Q", bits))[0]


def rn_text():
    r = rng.random()
    if r < 0.4:
        return repr(rn_double()).replace("inf", rng.choice(["inf", "Infinity", "INF"]))
    return rng.choice(["", "+", "-", ".", "1.", ".5", "1e", "e5", "1e5", "1E-5", "1.e5", ".e5", "+1.5", "-0", "inf", "-INF", "infinity", "InFiNiTy", "nan",
                       "-NaN", "NAN", "infinit", " 1", "1 ", "0x10", "1_0", "1e+", "1e400", "1e-400", "00001.000", "1e99999999999999999999", "+-1", "1.5.5",
                       "１", "2.4703282292062327e-324", "2.4703282292062328e-324", "179769313486231580793728971405303415079934132710037826936173778980444968292764750946649017977587207096330286416692887910946555547851940402630657488671505820681908902000708383676273854845817711531764475730270069855571366959622842914819860834936475292719074168444365510704342711559699508093042880177904174497792", "0." + "0" * 400 + "1"])


RN_INTS = ["0", "-0", "+0", "", "+", "-", "1", "-1", "127", "128", "-128", "-129", "255", "256", "32767", "-32768", "65535", "2147483647", "2147483648",
           "-2147483648", "4294967295", "4294967296", "9223372036854775807", "9223372036854775808", "-9223372036854775808", "-9223372036854775809",
           "18446744073709551615", "18446744073709551616", "1_0", " 1", "1 ", "0x10", "١", "00012", "+-1", "--1", "12a", "99999999999999999999x"]


def rustnum_line():
    r = rng.random()
    b = struct.pack(">d", rn_double()).hex()
    if r < 0.15:
        return f"d {b}"
    if r < 0.3:
        return f"g {b}"
    if r < 0.45:
        return f"f {b} {rng.choice([0, 0, 1, 2, 3, 5, 10, 17, 20, 30])}"
    if r < 0.55:
        return f"e {b} {rng.choice('01')}"
    if r < 0.65:
        return f"x {b} {rng.choice([0, 1, 2, 3, 6, 16, 17, 25])} {rng.choice('01')}"
    if r < 0.85:
        return f"p {rn_text().encode().hex() or '-'}"
    t = rng.choice(RN_INTS) if rng.random() < 0.7 else str(rng.randrange(-2 ** 65, 2 ** 65))
    return f"i {t.encode().hex() or '-'} {rng.choice(['i8', 'i16', 'i32', 'i64', 'u8', 'u16', 'u32', 'u64'])}"


family("rustnum", "rustnum", [rustnum_line() for _ in range(int(os.environ.get("RUSTNUM_LINES", "8000")))],
       [rust_tool("rustnum", os.path.join(LIB, "rustnum/oracle.rs"))])


# --- Java's LocalDate, against Java 17 ------------------------------------------------------------------------

JD_YEARS = [2024, 2023, 2000, 1900, 1970, 0, -1, -4, 9999, 10000, -9999, -10000, 999999999, -999999999, 12345, 1, 4, 100, 400]


def jd_month_length(y, m):
    if m == 2:
        return 29 if y % 4 == 0 and (y % 100 != 0 or y % 400 == 0) else 28
    return 30 if m in (4, 6, 9, 11) else 31


def jd_date():
    y = rng.choice(JD_YEARS) if rng.random() < 0.4 else rng.randrange(-3000, 5000)
    m = rng.randrange(1, 13)
    d = rng.choice([29, 30, 31]) if rng.random() < 0.4 else rng.randrange(1, 29)
    return f"{y},{m},{min(d, jd_month_length(y, m))}"


def jd_text():
    r = rng.random()
    if r < 0.5:
        y = rng.choice(["2024", "0000", "1970", "9999", "+10000", "-0001", "-10000", "+999999999", "-999999999", "+1000000000", "10000", "+2024", "-0000",
                        "202", "2024a", "１２３４"]) if rng.random() < 0.4 else f"{rng.randrange(0, 10000):04d}"
        m = rng.choice(["01", "02", "12", "13", "00", "1", "+1", "-1", "1a"]) if rng.random() < 0.3 else f"{rng.randrange(1, 13):02d}"
        d = rng.choice(["01", "28", "29", "30", "31", "32", "00", "3", "+1"]) if rng.random() < 0.4 else f"{rng.randrange(1, 29):02d}"
        t = y + rng.choice(["-", "-", "-", "/", ""]) + m + rng.choice(["-", "-", "-", "/"]) + d
        return t + rng.choice(["", "", "", "x", "T10:00", " ", "Z"])
    if r < 0.6:
        return rng.choice(["", "-", "2024-", "2024-01", "2024-01-", "x" * 70 + "2024-01-01", "2024-01-01" + "é" * 60, "😀" * 40, "2024-01-01" + "😀" * 30,
                           "2024-01-01" + "x" * 54, "2024-01-01" + "x" * 55, "2024-01-01" + "x" * 53 + "😀", "2024-01-01" + "x" * 52 + "😀", "2024-01-01" + "x" * 53 + "😀x"])
    return rng.choice(["2024-02-29", "2023-02-29", "1900-02-29", "2000-02-29", "2024-04-31", "2024-06-31", "2024-09-31", "2024-11-31", "2024-12-31", "2024-01-31"])


def javalocaldate_line():
    r = rng.random()
    if r < 0.3:
        return f"parse {jd_text().encode().hex() or '-'}"
    if r < 0.38:
        return f"of {rng.choice(JD_YEARS + [1000000000, -1000000000])} {rng.choice([1, 2, 12, 13, 0, -1])} {rng.choice([1, 28, 29, 30, 31, 32, 0])}"
    if r < 0.45:
        return f"epoch {rng.choice([0, -1, 19000, -719528, 2932896, 365241780471, 365241780472, -365243219162, -365243219163, rng.randrange(-10 ** 9, 10 ** 9)])}"
    if r < 0.55:
        return f"info {jd_date()}"
    if r < 0.8:
        n = rng.choice([0, 1, -1, 30, 31, 365, -365, 1000, 10 ** 6, -(10 ** 6), 12, 13, 2 ** 31, 10 ** 10, 2 ** 62, -(2 ** 62), 9223372036854775807,
                        -9223372036854775808, 1317624576693539401, 1317624576693539402, 2000000000, rng.randrange(-10 ** 5, 10 ** 5)])
        return f"plus {jd_date()} {rng.choice('dwmy')} {n}"
    if r < 0.9:
        return f"with {jd_date()} {rng.choice('ymd')} {rng.choice([1, 2, 12, 13, 0, 28, 29, 30, 31, 32, 2024, 2023, -1, 999999999, 1000000000])}"
    return f"until {jd_date()} {jd_date()} {rng.choice(['DAYS', 'WEEKS', 'MONTHS', 'YEARS', 'DECADES', 'CENTURIES', 'MILLENNIA'])}"


family("javalocaldate", "javalocaldate", [javalocaldate_line() for _ in range(int(os.environ.get("JAVALOCALDATE_LINES", "8000")))],
       ["java", "-XX:-OmitStackTraceInFastThrow", "-cp", JAVA, "LocalDateOracle"])


# --- Perl's Data::Dumper, against Data::Dumper 2.179 (XS, Perl 5.34) ---------------------------------------------

PD_WORDS = ["a", "b", "key", "x y", "it's", "back\\slash", "", "0", "01", "-0", "10", "-5", "123456789", "1234567890", "-123456789",
            "_id", "a1", "1a", "$x", "@y", "\"q\"", "tab\there", "nl\n", "\x7f", "a-b", "Foo::Bar", "\x00", "\x001", "\x1b[0m", "007"]
PD_UNI = ["é", "naïve", "☺", "😀", "\u0085", "日本", " x", "ü1", "​"]
PD_CLASSES = ["Foo", "Foo::Bar", "O'Neil", "back\\slash", "main", "A::B::C"]


def pd_text():
    r = rng.random()
    if r < 0.6:
        return rng.choice(PD_WORDS)
    if r < 0.8:
        return "".join(rng.choice(PD_WORDS) for _ in range(rng.randrange(1, 3)))
    return "".join(chr(rng.randrange(0, 128)) for _ in range(rng.randrange(0, 6)))


def pd_hex(b):
    return b.hex() or "-"


class PdGen:
    def __init__(self):
        self.nodes = []
        self.containers = []
        self.scalars = []

    def new(self, kind, arg=""):
        self.nodes.append([kind, arg])
        return len(self.nodes) - 1

    def scalar(self, depth):
        r = rng.random()
        if r < 0.07:
            return self.new("u")
        if r < 0.10:
            return self.new(rng.choice("yn"))
        if r < 0.24:
            v = rng.choice([0, 1, -1, 7, 42, 999999999, 1234567890, 12345678901, -123456789, -1234567890, 2 ** 63 - 1, -2 ** 63,
                            rng.randrange(-10 ** 6, 10 ** 6)])
            return self.new("i", (v & (2 ** 64 - 1)).to_bytes(8, "big").hex())
        if r < 0.32:
            v = rng.choice([0.5, 21.0, -0.0, 0.0, float("inf"), -float("inf"), float("nan"), 1e21, 0.1 + 0.2, 1e-7, 123456789012345680.0, 5e-324, -2.5,
                            3.0, rng.uniform(-1000, 1000)])
            b = struct.pack(">d", v)
            if rng.random() < 0.1 and v != v:
                b = bytes([b[0] | 0x80]) + b[1:]
            return self.new("d", b.hex())
        if r < 0.50:
            t = pd_text()
            b = t.encode("latin-1") if rng.random() < 0.8 else bytes(rng.randrange(0, 256) for _ in range(rng.randrange(1, 5)))
            return self.new("b", pd_hex(b))
        if r < 0.60:
            t = rng.choice(PD_UNI) + (pd_text() if rng.random() < 0.5 else "") if rng.random() < 0.8 else pd_text()
            return self.new("s", pd_hex(t.encode("utf-8")))
        return self.new("r", str(self.ref_target(depth)))

    def bless(self, target):
        return self.new("B", pd_hex(rng.choice(PD_CLASSES).encode()) + "/" + str(target))

    def ref_target(self, depth):
        r = rng.random()
        if r < 0.15 and self.containers:
            return rng.choice(self.containers)
        if r < 0.22 and self.scalars:
            return rng.choice(self.scalars)
        if r < 0.25:
            return self.new("U")
        if depth <= 0 or r < 0.38:
            t = self.scalar(depth - 1)
            self.scalars.append(t)
            return self.bless(t) if rng.random() < 0.1 else t
        return self.container(depth - 1)

    def key(self, taken):
        while True:
            r = rng.random()
            if r < 0.7:
                k, utf8 = pd_text(), False
                b = k.encode("latin-1")
            elif r < 0.85:
                k = rng.choice(PD_UNI) + (rng.choice(PD_WORDS) if rng.random() < 0.4 else "")
                utf8, b = True, k.encode("utf-8")
            else:
                b = bytes(rng.randrange(0, 256) for _ in range(rng.randrange(1, 4)))
                k, utf8 = b.decode("latin-1"), False
            # Perl copies the path to an array with strlcpy, which stops at a
            # NUL; the rest of the name is uninitialized memory. A NUL in a
            # key is set aside for that reason.
            if k not in taken and 0 not in b:
                taken.add(k)
                return pd_hex(b), utf8

    def container(self, depth):
        me = self.new("a" if rng.random() < 0.5 else "h", "-")
        self.containers.append(me)
        n = rng.choice([0, 1, 2, 3, 3, 5])
        if self.nodes[me][0] == "a":
            items = [self.scalar(depth) for _ in range(n)]
            self.scalars.extend(items)
            self.nodes[me][1] = ".".join(map(str, items)) or "-"
        else:
            taken, parts = set(), []
            for _ in range(n):
                kh, utf8 = self.key(taken)
                v = self.scalar(depth)
                self.scalars.append(v)
                parts.append(f"{kh}/{int(utf8)}/{v}")
            self.nodes[me][1] = ".".join(parts) or "-"
        if rng.random() < 0.15:
            b = self.bless(me)
            self.containers.append(b)
            return b
        return me


def perldumper_line():
    g = PdGen()
    depth = rng.choice([1, 2, 3, 3, 4])
    roots = []
    for _ in range(rng.choice([1, 1, 1, 2, 3])):
        r = rng.random()
        if r < 0.15 and g.containers:
            roots.append(rng.choice(g.containers))
        elif r < 0.75:
            roots.append(g.container(depth))
        else:
            roots.append(g.scalar(depth))
    names = rng.choice([None] * 4 + [["x"], ["*x"], ["*a", "*b", "$c"], ["", "y"], ["$z", "*q", "*r"], ["*s"]])
    o = [str(rng.choice([0, 1, 2, 2, 3])), pd_hex(rng.choice(["", "", "", "# ", "  "]).encode()), pd_hex(rng.choice(["VAR", "VAR", "x", "Foo_"]).encode()),
         str(int(rng.random() < 0.2)), str(int(rng.random() < 0.3)), str(int(rng.random() < 0.8)),
         pd_hex(rng.choice([" => ", " => ", ": ", "=>", ","]).encode()), str(int(rng.random() < 0.2)), str(int(rng.random() < 0.25)),
         str(int(rng.random() < 0.2)), str(rng.choice([1000, 1000, 1000, 0, 1, 2, 3])), pd_hex(rng.choice(["bless", "bless", "Foo::bless"]).encode()),
         "-" if names is None else ".".join(pd_hex(n.encode()) for n in names)]
    nodes = ";".join(k + (":" + a if a else "") for k, a in g.nodes)
    return f"perldumper {','.join(o)} {nodes} {'.'.join(map(str, roots))}"


family("perldumper", "perldumper", [perldumper_line() for _ in range(int(os.environ.get("PERLDUMPER_LINES", "6000")))],
       ["perl", os.path.join(LIB, "perldumper/oracle.pl")])


# --- SQLite's printf, against SQLite 3.51.1 ---------------------------------------------------------------------

SP_TEXTS = ["", "abc", "it's", "back\\slash", "  12abc", "1e3", "0x10", "-", "-0", "007", "12.5e-1", "é", "naïve", "日本語", "😀x", "a\x01b",
            "\x1f", "say \"hi\"", "9223372036854775808", "-9223372036854775809", "99999999999999999999", " 3.25 ", "inf", "1e400", "-1e-400",
            "a\x00b", "\t7", "+5", ".5", "5.", "e5", "1e", "1e+", "\x01", "line\nnext", "\x1b[0m", "x\x7fy", "c:\\dir\\", "'\x02'",
            "1234567890123456789", "123456789012345678", "9223372036854775807", "-9223372036854775808", "  -0042"]
SP_REALS = [0.0, -0.0, 1.0, -1.5, 0.1, 2.5, 0.125, 1e15, 1e16, 123456.789, 1e100, 1e-100, 5e-324, 1.7976931348623157e308, float("inf"), -float("inf"),
            0.5, 1.0 / 3, 99.995, 9.9999995, 1e21, -123.456e-7, 4.35, 2 ** 53, 0.000123456]
# Widths and precisions come from these too, through `*`: SQLite fails a
# field past its length limit ("string or blob too big"), so the values are
# kept to ones whose low 32 bits are a small int (2**32 + 3 is 3, 2**63 - 1
# is -1, 2**32 - 40 is -40).
SP_INTS = [0, 1, -1, 7, 12, 113, 255, 1000, 12345, -12345, 2 ** 32 + 3, 2 ** 32 - 40, 2 ** 63 - 1, -2 ** 63, 21, 22, 23, 111, 112, 4294967296, -7]


def sp_value(kinds="ziintb"):
    k = rng.choice(kinds)
    if k == "z":
        return "z"
    if k == "i":
        return "i" + str(rng.choice(SP_INTS) if rng.random() < 0.7 else rng.randrange(-3000, 3000))
    if k == "n":
        v = rng.choice(SP_REALS) if rng.random() < 0.7 else rng.uniform(-1e6, 1e6) * 10 ** rng.randrange(-8, 8)
        return "n" + struct.pack(">d", v).hex()
    if k == "t":
        t = rng.choice(SP_TEXTS) if rng.random() < 0.8 else "".join(rng.choice(SP_TEXTS) for _ in range(2))
        return "t" + (t.encode().hex() or "-")
    return "b" + (bytes(rng.randrange(0, 256) for _ in range(rng.randrange(0, 5))).hex() or "-")


def sp_star():
    """An argument for `*`: one whose low 32 bits are a small int, so the
    field stays under SQLite's length limit."""
    r = rng.random()
    if r < 0.6:
        return "i" + str(rng.choice([0, 1, 3, 7, 12, -5, -1, 40, 2 ** 32 + 3, 2 ** 63 - 1, -2 ** 63, -2 ** 31]))
    if r < 0.8:
        return "t" + rng.choice(["5", "  7x", "-3", "", "abc", "1e2"]).encode().hex()
    if r < 0.95:
        return "n" + struct.pack(">d", rng.choice([2.5, -3.9, 0.0, 7.0, 1e300])).hex()
    return "z"


def sp_spec():
    flags = "".join(rng.choice("-+ #!0,") for _ in range(rng.choice([0, 0, 1, 1, 2, 3])))
    width = rng.choice(["", "", "", "5", "1", "12", "*", "007", "30", "2"])
    prec = rng.choice(["", "", "", ".0", ".1", ".3", ".10", ".*", ".", ".20", ".2"])
    size = rng.choice(["", "", "", "", "l", "ll"])
    conv = rng.choice("diuxXoprfeEgGszcqQwn%" * 3 + "yTS")
    args = ([sp_star()] if width == "*" else []) + ([sp_star()] if prec == ".*" else [])
    if conv not in "n%yTS":
        args.append(sp_value("ttttzib" if conv in "qQwszc" else "ziintb"))
    return "%" + flags + width + prec + size + conv, args


def sqliteprintf_line():
    parts, args = [], []
    for _ in range(rng.choice([1, 1, 2, 3])):
        if rng.random() < 0.3:
            parts.append(rng.choice(["x", "é", " | ", "'", "", "ab"]))
        spec, more = sp_spec()
        parts.append(spec)
        args += more
    if rng.random() < 0.05:
        parts.append(rng.choice(["%", "%5", "%.", "%l", "%!", "%-"]))
    if rng.random() < 0.1 and args:
        args = args[:rng.randrange(len(args))]
    elif rng.random() < 0.1:
        args.append(sp_value())
    fmt = "".join(parts)
    fv = "t" + (fmt.encode().hex() or "-") if rng.random() < 0.95 else sp_value("zinb")
    return " ".join(["sqliteprintf", fv] + args)


family("sqliteprintf", "sqliteprintf", [sqliteprintf_line() for _ in range(int(os.environ.get("SQLITEPRINTF_LINES", "12000")))],
       ["python3.13", os.path.join(LIB, "sqliteprintf/oracle.py")])


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
