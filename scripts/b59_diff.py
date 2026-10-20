"""B59's packages against the references they reproduce.

- Ruby 2.6's Abbrev.abbrev, with no pattern and with a String prefix.
- Ruby 2.6's Integer: to_s(base), digits, Integer.sqrt, pow(b, m), bit_length,
  gcd, lcm, round/floor/ceil/truncate with ndigits and half:, int[i],
  divmod and remainder.
- Node 25's String.prototype split, padStart/padEnd, at, codePointAt,
  isWellFormed/toWellFormed, trim/trimStart/trimEnd, substring, substr,
  slice, indexOf, lastIndexOf, includes, startsWith, endsWith, repeat,
  replace and replaceAll with string patterns, on UTF-16 code units.
- Ruby 2.6's Time#strftime on UTC times: every conversion, flags, widths,
  %N precision, the colon forms of %z, and the invalid-format error.
- Ruby 2.6's Kernel#format: every conversion, flag, width, precision, *,
  N$ and named reference, with Ruby's exceptions.
- Ruby 2.6's json 2.1: JSON.generate, JSON.pretty_generate and JSON.parse,
  error classes and messages included.
- Node 25's Date with TZ=UTC: Date.parse (ISO and V8's legacy parser),
  Date.UTC, TimeClip, toISOString, toUTCString, toString, toDateString,
  toTimeString and the UTC getters.
- Node 25's legacy url.parse (both options), url.format and url.resolve.
- Ruby 2.6's URI.parse (with the scheme classes), URI#to_s, URI.join and
  the www-form functions.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B59_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B59_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B59_LIB") or os.path.join(ROOT, "scripts/lib/b59")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b59")
harness = os.path.join(work, "b59test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b59test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


# --- Ruby's Abbrev.abbrev, against Ruby 2.6 ---------------------------------------------------------------

AB_WORDS = ["car", "cart", "cone", "con", "c", "", "a", "ab", "abc", "abd", "b", "ruby", "rules", "run", "rune", "é", "éa", "éb", "日本", "日本語",
            "😀", "😀x", "x", "xy", "xyz", "help", "hello", "hell", "he", "h", "A", "Ab", "aB"]


def ab_word():
    r = rng.random()
    if r < 0.6:
        return rng.choice(AB_WORDS)
    return "".join(rng.choice("abcé😀A") for _ in range(rng.randrange(0, 5)))


def rubyabbrev_line():
    words = [ab_word() for _ in range(rng.choice([0, 1, 2, 2, 3, 3, 4, 5, 6, 8, 10]))]
    if rng.random() < 0.15 and words:
        words.append(rng.choice(words))
    r = rng.random()
    if r < 0.45:
        pattern = "~"
    else:
        p = rng.choice(["", "c", "ca", "a", "ab", "r", "ru", "é", "日", "😀", "h", "he", "hel", "x", "z", ".", "A"]) if r < 0.65 else ab_word()
        if r >= 0.65 and r < 0.85 and words:
            w = rng.choice(words)
            p = w[:rng.randrange(len(w) + 1)]
        pattern = p.encode().hex() or "-"
    return pattern + (" " + " ".join(w.encode().hex() or "-" for w in words) if words else "")


family("rubyabbrev", "rubyabbrev", [rubyabbrev_line() for _ in range(int(os.environ.get("RUBYABBREV_LINES", "10000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubyabbrev/oracle.rb")])

# --- Ruby's Integer, against Ruby 2.6 -----------------------------------------------------------------------------

def ri_int():
    r = rng.random()
    if r < 0.25:
        v = rng.choice([0, 1, 2, 3, 5, 7, 9, 10, 15, 25, 35, 45, 50, 95, 99, 100, 150, 250, 255, 256, 999, 1000, 1500, 2 ** 31, 2 ** 32, 2 ** 62 - 1,
                        2 ** 62, 2 ** 62 + 1, 2 ** 62 - 10, 2 ** 63 - 1, 2 ** 63, 2 ** 64, 2 ** 64 - 1, 10 ** 18, 10 ** 19, 10 ** 20, 10 ** 21, 4611686018427387900,
                        4611686018427387910, 5 * 10 ** 18, 25 * 10 ** 18, 15 * 10 ** 19, 2 ** 100, 2 ** 200, 10 ** 40, 5 * 10 ** 39])
    elif r < 0.45:
        k = rng.randrange(1, 45)
        v = rng.randrange(1, 100) * 10 ** k + rng.choice([0, 0, 0, 1, -1, 5 * 10 ** (k - 1), 5 * 10 ** (k - 1) + 1, 5 * 10 ** (k - 1) - 1, rng.randrange(10 ** k)])
    elif r < 0.7:
        v = rng.randrange(0, 10 ** rng.randrange(1, 4))
    else:
        v = rng.randrange(0, 2 ** rng.randrange(1, 220))
    return -v if rng.random() < 0.45 else v


def ri_nd():
    r = rng.random()
    if r < 0.8:
        return rng.randrange(-24, 3)
    if r < 0.93:
        return rng.randrange(-90, -24)
    return rng.choice([-2 ** 31, -2 ** 31 + 1, 2 ** 31 - 1, 2 ** 31, -2 ** 31 - 1, 2 ** 40, -2 ** 40, -10 ** 6])


def rubyinteger_line():
    r = rng.random()
    a = ri_int()
    if r < 0.1:
        return f"ts {a} {rng.randrange(2, 37) if rng.random() < 0.6 else rng.choice([0, 1, 37, -2, -16, 2 ** 31 - 1, -2 ** 31, 2 ** 31, -2 ** 31 - 1, 2 ** 40, 100])}"
    if r < 0.18:
        base = rng.choice(["~", 2, 10, 16, 3, 7, 100, 1000, 256, 10 ** 9, 2 ** 62 - 1, 2 ** 62, 2 ** 64, 10 ** 25, 0, 1, -1, -2, -10, -2 ** 64])
        return f"dg {abs(a) if rng.random() < 0.85 else a} {base}"
    if r < 0.24:
        if rng.random() < 0.5:
            m = rng.choice([rng.randrange(2 ** 26, 94906266), rng.randrange(1, 2 ** rng.randrange(1, 120))])
            return f"sq {m * m + rng.choice([0, -1, -2, 1, -m])}"
        return f"sq {abs(a) if rng.random() < 0.9 else a}"
    if r < 0.34:
        b = rng.choice([0, 0, 1, 2, 3, 5, 10, 65537, 2 ** 64 + 1, rng.randrange(0, 10 ** 30), -1, -5])
        m = rng.choice([1, -1, 2, -2, 3, 7, -7, 10, 1000, 2 ** 31 - 1, 2 ** 32 + 15, 2 ** 62 - 57, -(2 ** 62), 2 ** 62, 2 ** 64 + 13, -(2 ** 64 + 13), 10 ** 25 + 7, 0,
                        ri_int()])
        return f"pm {a} {b} {m}"
    if r < 0.4:
        return f"bl {a}"
    if r < 0.47:
        return f"gl {a} {rng.choice([0, 1, -1, 2, 6, 10, -12, 2 ** 64, ri_int(), ri_int()])}"
    if r < 0.85:
        nd = ri_nd()
        if rng.random() < 0.5 and -60 < nd < 0:
            # A tie or an exact multiple at the digit being rounded, sometimes zero.
            k = -nd
            a = rng.choice([0, 1, rng.randrange(1, 100), rng.randrange(1, 10 ** rng.randrange(1, 30))]) * 10 ** k + rng.choice([0, 0, 5 * 10 ** (k - 1), 5 * 10 ** (k - 1), 1, -1])
            a = -a if rng.random() < 0.5 else a
        if r >= 0.67:
            if rng.random() < 0.08:
                nd, a = rng.choice([-18, -19, -20]), rng.choice([0, 0, 10 ** 19, -10 ** 19, 2 * 10 ** 18, -4 * 10 ** 18])
            return f"{rng.choice(['fl', 'ce', 'tr'])} {a} {nd}"
        mode = rng.choice(["~", "~", "~", "up", "up", "even", "even", "down", "down", "UP", "Even", "dOwN", "", "upx", "bogus", "half", "évén"])
        return f"rd {a} {nd} {mode if mode == '~' else mode.encode().hex() or '-'}"
    if r < 0.92:
        return f"ba {a} {rng.choice([0, 1, 2, 7, 62, 63, 64, 65, 100, 199, 200, 1000, -1, -64, 2 ** 64, -(2 ** 64), rng.randrange(0, 230)])}"
    return f"dm {a} {rng.choice([0, 1, -1, 2, -2, 3, 7, -7, 10, 2 ** 62, -(2 ** 64), ri_int(), ri_int()])}"


family("rubyinteger", "rubyinteger", [rubyinteger_line() for _ in range(int(os.environ.get("RUBYINTEGER_LINES", "10000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubyinteger/oracle.rb")])


# --- JavaScript's String.prototype methods, against Node 25 -------------------------------------------------------

JS_UNIT_PIECES = [[0x61], [0x62], [0x63], [0x2c], [0x20], [0x24], [0x26], [0x60], [0x27], [0x31], [0x3c], [0xe9], [0xd83d, 0xde00], [0xd83d], [0xde00],
                  [0xdbff, 0xdfff], [0x9], [0xa], [0xb], [0xc], [0xd], [0xa0], [0x1680], [0x2000], [0x200a], [0x200b], [0x2028], [0x2029], [0x202f],
                  [0x205f], [0x3000], [0xfeff], [0x180e], [0x85], [0x0]]


def js_units(n=None):
    out = []
    for _ in range(rng.randrange(0, 7) if n is None else n):
        out += rng.choice(JS_UNIT_PIECES[:12] if rng.random() < 0.5 else JS_UNIT_PIECES)
    return out


def js_hex(us):
    return "".join(f"{u:04x}" for u in us) or "-"


def js_num():
    r = rng.random()
    if r < 0.12:
        return "u"
    if r < 0.6:
        return str(rng.randrange(-8, 10))
    return rng.choice(["nan", "inf", "-inf", "-0.0", "0.5", "-0.5", "1.5", "2.9", "-1.5", "-2.9", "1e+20", "-1e+20", "4294967295", "4294967296", "4294967297",
                       "-4294967295", "-1", "8589934593", "1e-07", "9007199254740993", "3.0000000001"])


def js_sub(s):
    if s and rng.random() < 0.6:
        i = rng.randrange(len(s) + 1)
        return s[i:i + rng.randrange(0, 4)]
    return js_units(rng.randrange(0, 3))


def js_search_line(op, s):
    return f"{op} {js_hex(s)} {js_hex(js_sub(s))} {js_num()}"


def jsstringmethods_line():
    s = js_units()
    op = rng.choice(["sp", "sp", "ps", "pe", "at", "cp", "wf", "tm", "tm", "sb", "sr", "sl", "io", "li", "li", "ic", "sw", "ew", "rp", "re", "ra", "ra"])
    if op == "sp":
        sep = "u" if rng.random() < 0.1 else js_hex(js_sub(s))
        return f"sp {js_hex(s)} {sep} {js_num() if rng.random() < 0.5 else 'u'}"
    if op in ("ps", "pe"):
        # Huge lengths only where V8 refuses them: a result at the limit is 512 MB.
        n = rng.choice([js_num(), str(len(s) + rng.randrange(-2, 9)), str(rng.choice([536870889, 536870890, 2 ** 31, 2 ** 53, 10 ** 300]))])
        fill = rng.choice(["u", js_hex(js_units(rng.randrange(0, 3)))])
        return f"{op} {js_hex(s)} {n} {fill}"
    if op in ("at", "cp"):
        return f"{op} {js_hex(s)} {js_num() if rng.random() < 0.6 else str(rng.randrange(-len(s) - 1, len(s) + 1))}"
    if op in ("wf", "tm"):
        return f"{op} {js_hex(s)}"
    if op in ("sb", "sr", "sl"):
        return f"{op} {js_hex(s)} {js_num()} {js_num()}"
    if op in ("io", "li", "ic", "sw", "ew"):
        return js_search_line(op, s)
    if op == "rp":
        s = s[:3]
        n = rng.choice([js_num(), js_num(), str(rng.randrange(0, 5))])
        if rng.random() < 0.2:
            # Past V8's 536870888 units, or of an empty string; never a result at the limit.
            s = rng.choice([[], [0x61], [0x61, 0x62], [0x61, 0x62, 0x63]])
            n = str(rng.choice([536870888 // len(s) + 1, 536870888 // len(s) + 2, 2 ** 40, 10 ** 300]) if s else rng.choice([536870888, 2 ** 40, 10 ** 300]))
        return f"rp {js_hex(s)} {n}"
    repl = []
    for _ in range(rng.randrange(0, 4)):
        repl += rng.choice([[0x24, 0x24], [0x24, 0x26], [0x24, 0x60], [0x24, 0x27], [0x24, 0x31], [0x24, 0x30], [0x24, 0x3c, 0x78, 0x3e], [0x24], [0x78], [0xd83d],
                            [0x24, 0x31, 0x32], [0x24, 0x24, 0x26]])
    return f"{op} {js_hex(s)} {js_hex(js_sub(s))} {js_hex(repl)}"


family("jsstringmethods", "jsstringmethods", [jsstringmethods_line() for _ in range(int(os.environ.get("JSSTRINGMETHODS_LINES", "12000")))],
       ["node", os.path.join(LIB, "jsstringmethods/oracle.js")])


# --- Ruby's Time#strftime, against Ruby 2.6 -----------------------------------------------------------------------

SF_CONV = list("YCymBbhdejHkIlPpMSLNzZAauwGgVUWsntcDFvxXrRT%") + ["Q", "+", "q", "E", "O", "J", "é", "f", "K", "i"]


def sf_epoch():
    import calendar
    r = rng.random()
    if r < 0.3:
        return rng.randrange(-2 ** 31, 2 ** 32)
    if r < 0.55:
        # Around the turn of a year, where the week numbers move.
        y = rng.choice([rng.randrange(1, 3000), rng.randrange(1900, 2100), rng.randrange(-3000, 1), rng.choice([0, -1, -100, -101, -400, 1600, 2000, 2100])])
        days = rng.choice([0, 1, 2, 3, 4, 5, 6, -1, -2, -3, -4, -5, -6, -7, 59, 60, 365, 364])
        base = (y - 1970) * 365 + (y - 1969) // 4 - (y - 1901) // 100 + (y - 1601) // 400
        return (base + days) * 86400 + rng.choice([0, 1, 43199, 43200, 46800, 3600, 86399, rng.randrange(86400)])
    if r < 0.85:
        return rng.randrange(-62167219200 - 86400 * 365 * 2000, 253402300800 * 3)
    return rng.choice([0, -1, 1, 86399, 86400, -86400, -86401, 951782400, 951868800, 10 ** 12, -10 ** 12, 10 ** 13, -10 ** 13, 253402300799, -62135596800, -62135596801])


def sf_directive():
    r = rng.random()
    flags = "".join(rng.choice("-_0^#") for _ in range(rng.choice([0, 0, 0, 1, 1, 2])))
    width = rng.choice(["", "", "", "", str(rng.randrange(0, 13)), "0" + str(rng.randrange(0, 10)), str(rng.randrange(13, 40)), "2147483648", "99999999999"])
    mod = rng.choice([""] * 12 + ["E", "O", ":", "::", ":::", "::::", ":::::"])
    conv = rng.choice(SF_CONV + ["z", "z", "z", "N", "N", "L", "c"])
    if r < 0.05:
        return "%" + width + flags + conv
    return "%" + flags + width + mod + conv


def rubystrftime_line():
    r = rng.random()
    if r < 0.15:
        # Week numbering and calendar fields where they turn over, with flags.
        fmt = rng.choice(["%G-W%V-%u", "%g %U %W %j", "%F %a", "%Y %C %y %G %g", "%-V %_U %0W", "%j|%b %e", "%D %x", "%0-30c", "%0-12F",
                          "%_-12T", "%0-9A", "%-030c"])
        y = rng.choice([1700, 1800, 1900, 2000, 2100, 2200, 1600, 400, 100, -100, -400, -1, 0, 1, rng.randrange(-500, 2500)])
        base = (y - 1970) * 365 + (y - 1969) // 4 - (y - 1901) // 100 + (y - 1601) // 400
        days = rng.choice([rng.randrange(-8, 8), rng.randrange(55, 62), rng.randrange(355, 368)])
        return f"{(base + days) * 86400 + rng.randrange(86400)} {rng.randrange(10 ** 9)} {fmt.encode().hex()}"
    if r < 0.19:
        fmt = rng.choice(["", "%", "abc%", "%-", "%5", "%_", "%^", "%#", "%0", "%E", "%O", "%:", "%::", "%:::", "x%10", "%10-d", "%::::d", "%::::%",
                          "%:::::z", "%::::z", "%-10d", "%-30c", "%_30c", "%030c", "%^30c", "%#30c"])
    else:
        parts = []
        for _ in range(rng.randrange(1, 5)):
            parts.append(sf_directive() if rng.random() < 0.75 else rng.choice(["-", " ", "T", ":", "é", "x", "%%", "\t", "日"]))
        fmt = "".join(parts)
        if rng.random() < 0.05:
            fmt += "%"
    nsec = rng.choice([0, 0, 5, 50, 123456789, 999999999, 1000000, rng.randrange(10 ** 9)])
    return f"{sf_epoch()} {nsec} {fmt.encode().hex() or '-'}"


family("rubystrftime", "rubystrftime", [rubystrftime_line() for _ in range(int(os.environ.get("RUBYSTRFTIME_LINES", "12000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubystrftime/oracle.rb")])


# --- Ruby's Kernel#format, against Ruby 2.6 -----------------------------------------------------------------------

def rf_float():
    r = rng.random()
    if r < 0.25:
        return struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]
    if r < 0.55:
        return rng.choice([0.0, -0.0, 0.5, 1.5, 2.5, -2.5, 0.125, 0.375, 1.005, 2.675, 0.1, 0.3, 1e-5, 9.5e-5, 123.456, 1e21, 1e22, 1e15, 1e16, 1e17,
                           999999.5, 9.9999995, 0.000123, 5e-324, 2.2250738585072014e-308, 1.7976931348623157e308, 1 / 3, 2 / 3, 100.0, 1e6, 1e-4,
                           0.0001, 99999.95, 1e100, 4.35, 1.25, 0.05, 1.0 + 2 ** -52, 2.0 - 2 ** -52, 1.96875, 1.03125, 3.0, -1.0])
    if r < 0.62:
        return rng.choice([float("nan"), float("inf"), float("-inf")])
    if r < 0.8:
        # A tie at some decimal place: k/2^n is exact.
        return rng.randrange(-10 ** 6, 10 ** 6) / 2 ** rng.randrange(0, 12)
    return rng.uniform(-1, 1) * 10 ** rng.randrange(-30, 30)


def rf_ftok(x):
    return "f:" + struct.pack(">d", x).hex()


def rf_int():
    r = rng.random()
    if r < 0.4:
        return rng.randrange(-300, 300)
    if r < 0.6:
        return rng.choice([0, -1, 1, 255, -255, -256, 256, 8, -8, 2 ** 31 - 1, -2 ** 31, 2 ** 31, 2 ** 62, -2 ** 62, 2 ** 63, -2 ** 63 - 1, 2 ** 64, -2 ** 64, -2 ** 64 - 1, 10 ** 30, -10 ** 30])
    return rng.randrange(-2 ** rng.randrange(1, 140), 2 ** rng.randrange(1, 140))


RF_STRS = ["", "a", "abc", "héllo", "日本語", "😀x", "x y", "a\"b\\c#{d}#$e#@f#g", "\n\t\x1b\x01\x7f", "%", "0123456789"]


def rf_arg(conv):
    r = rng.random()
    if conv in "diuoxXbB":
        return "i:%d" % rf_int() if r < 0.75 else rf_ftok(rf_float()) if r < 0.97 else rng.choice(["n", "t", "F"])
    if conv == "f":
        return "i:%d" % rf_int() if r < 0.25 else rf_ftok(rf_float()) if r < 0.97 else rng.choice(["n", "t"])
    if conv in "eEgGaA":
        return rf_ftok(rf_float()) if r < 0.85 else "i:%d" % rng.choice([rf_int(), 2 ** 1024, -2 ** 1100, 2 ** 1023 * 2 - 2 ** 970, 2 ** 53 + 1]) if r < 0.97 else "n"
    if conv == "c":
        if r < 0.45:
            return "i:%d" % rng.choice([65, 97, 48, 32, 0, 233, 0x263a, 0x1f600, 0x10ffff, 0xd7ff, 0xe000, 0x110000, 0x7fffffff, 2 ** 31, -3, -128, -2 ** 31, -2 ** 31 - 1, rng.randrange(0, 0xd800)])
        if r < 0.8:
            return "s:" + (rng.choice(["a", "é", "😀", "%", " ", "ab", "", "日本"]).encode().hex() or "-")
        return rng.choice([rf_ftok(65.7), rf_ftok(1e20), rf_ftok(float("nan")), "n", "t"])
    if conv == "p":
        text = rng.choice(["".join(rng.choice("ab\"\\#{$@}\n\t\x1b\x01\x7f ") for _ in range(rng.randrange(0, 6))), "#$x", "#@y", "#{z}", "##", "#a", "a#"])
        return rng.choice(["i:%d" % rf_int(), "n", "t", "F", "s:" + (text.encode().hex() or "-"), "s:" + (text.encode().hex() or "-")])
    return rng.choice(["i:%d" % rf_int(), "n", "t", "F"] + ["s:" + (rng.choice(RF_STRS).encode().hex() or "-")] * 3)


def rf_star_arg():
    # Not -2**31: negating it is undefined behaviour in sprintf.c, and Ruby 2.6 pads to 2**32.
    if rng.random() < 0.75:
        return "i:%d" % rng.randrange(-12, 25)
    return rng.choice(["i:%d" % rng.randrange(-12, 25), "i:%d" % rng.randrange(-12, 25), "i:2147483648", "i:-2147483649", rf_ftok(2.9), rf_ftok(-2.9),
                       rf_ftok(1e10), rf_ftok(1e20), rf_ftok(float("nan")), rf_ftok(float("-inf")), "n", "t", "s:33"])


RF_CONVS = "diuoxXbBfeEgGaAcsp"


def rf_fits(tok, conv):
    """Whether this argument is in the domain the package converts for conv:
    no Float to %s/%p, Strings only to %s/%p/%c (ASCII for %p), and no %c of
    -1, -2 or a surrogate, which Ruby turns into bytes that are not UTF-8."""
    if tok.startswith("f:"):
        x = struct.unpack(">d", bytes.fromhex(tok[2:]))[0]
        return conv not in "sp{" and not (conv == "c" and -3 < x <= -1)
    if tok.startswith("s:"):
        raw = bytes.fromhex(tok[2:]) if tok[2:] != "-" else b""
        return conv in "sc{" or conv == "p" and raw.isascii()
    if tok.startswith("i:") and conv == "c":
        v = int(tok[2:])
        return v not in (-1, -2) and not 0xd800 <= v <= 0xdfff
    return True


def rf_conv_for(tok):
    return rng.choice([c for c in RF_CONVS + "dxfegs" if rf_fits(tok, c)])


def rf_spec(mode, args, conv):
    """One directive; appends the arguments it consumes in unnumbered mode."""
    flags = "".join(rng.choice("-+ 0#") for _ in range(rng.choice([0, 0, 0, 1, 1, 2, 3])))
    width = rng.choice(["", "", "", str(rng.randrange(1, 12)), str(rng.randrange(12, 40)), "*"])
    prec = rng.choice(["", "", "", "." + str(rng.randrange(0, 8)), "." + str(rng.randrange(8, 25)), ".", ".*"])
    if conv in "eEfgGaA" and rng.random() < 0.05:
        prec = "." + str(rng.choice([30, 60, 100, 340, 1030, 1100]))
    if mode == "pos":
        # Arguments 1-4 are values, 5 and 6 bounded widths (a width taken
        # from an arbitrary value can be gigabytes of padding), 7 is past
        # the end.
        if width == "*":
            width = "*%d$" % rng.choice([5, 6, 5, 6, 7, 1])
        if prec == ".*":
            prec = ".*%d$" % rng.choice([5, 6, 5, 6, 7, 1])
        n = rng.choice([1, 2, 3, 4, 1, 2, 3, 4, 2, 3, 4, 5, 7])
        if n <= len(args):
            conv = rf_conv_for(args[n - 1])
        return "%" + "%d$" % n + flags + width + prec + conv
    if mode == "hash":
        width = width.replace("*", "5")
        prec = prec.replace("*", "2")
        name = rng.choice(["a", "b", "c", "a", "b", "c", "zz"])
        tok = args[0].get(name) if args else None
        if rng.random() < 0.3 and (tok is None or rf_fits(tok, "{")):
            return "%" + flags + width + prec + "{" + name + "}"
        if tok is not None:
            conv = rf_conv_for(tok)
        return "%" + flags + "<" + name + ">" + width + prec + conv
    if width == "*":
        args.append(rf_star_arg())
    if prec == ".*":
        args.append(rf_star_arg())
    args.append(rf_arg(conv))
    return "%" + flags + width + prec + conv


RF_ODD = ["%", "abc%", "%-", "%5", "%.", "%.5", "%*", "%y", "%$", "%é", "%\x01", "%5%", "%-%", "%\n", "% \n", "%1$", "%<a", "%{a", "%0$s", "%*0$d", "%1$1$s",
          "%5*d", "%.5.3d", "%.5 d", "%5 d", "%5.3*d", "%*1$d", "%1$*d", "%2147483648d", "%.2147483648d", "%*2147483648$d", "%.*2147483648$d",
          "%99999999999$d", "%1$s %s", "%s %1$s", "%<a>s %s", "%s %<a>s", "%1$s %<a>s", "%<a>s %1$s", "%<a><b>s", "%<a>{b}", "%<a>", "%{a}%<a>s",
          "%c", "%s", "%d %d %d"]


def rf_near_half():
    """A value near a rounding tie for the precision it is printed at:
    a decimal ending in 5 (the double nearest it is a hair either side),
    or a binary fraction cut exactly at half a hex digit."""
    if rng.random() < 0.3:
        bits = rng.randrange(1, 13)
        p = rng.randrange(0, (bits + 3) // 4 + 1)
        x = (1 + rng.randrange(1 << bits) / (1 << bits)) * 2.0 ** rng.randrange(-20, 20)
        return "%" + rng.choice(["", "#", "+", "0", "-"]) + rng.choice(["", "12"]) + ".%da" % p, x
    digits = rng.randrange(1, 16)
    p = rng.randrange(0, digits)
    text = "".join(rng.choice("0123456789") for _ in range(digits - 1)) + "5"
    x = float(text[:digits - p - 1] + "." + text[digits - p - 1:]) if digits - p - 1 > 0 else float("0." + "0" * (p + 1 - digits) + text)
    conv = rng.choice("feEgG")
    if conv == "f":
        prec = p
    else:
        # Significant digits up to the 5, minus one for %e.
        sig = len(("%.17e" % x).split("e")[0].replace(".", "").lstrip("0")) if x else 1
        lead = len(text.lstrip("0")) - 1
        prec = max(lead - (1 if conv in "eE" else 0), 0) if rng.random() < 0.8 else rng.randrange(0, 18)
    return "%" + rng.choice(["", "", "#", "+", " "]) + ".%d%s" % (prec, conv), x * rng.choice([1, 1, -1])


def rubyformat_line():
    r = rng.random()
    args = []
    if r < 0.12:
        fmt, x = rf_near_half()
        return f"{fmt.encode().hex()} {rf_ftok(x)}"
    r = rng.random()
    if r < 0.08:
        fmt = rng.choice(RF_ODD)
        args = rng.choice([[], ["i:1"], ["i:1", "i:2"], ["h:61=i:1,62=i:2"], ["h:61=s:78797a"], ["i:3", "n"]])
        return " ".join([fmt.encode().hex() or "-"] + args)
    mode = "seq" if r < 0.72 else "pos" if r < 0.85 else "hash"
    if mode == "pos":
        # Argument 1 doubles as a width now and then, so it stays small.
        args = [rf_star_arg()] + [rf_arg(rng.choice(RF_CONVS)) for _ in range(3)] + [rf_star_arg(), rf_star_arg()]
    elif mode == "hash":
        args = [{k: rf_arg(rng.choice(RF_CONVS)) for k in ["a", "b", "c"] if rng.random() < 0.85}]
    pieces = []
    for _ in range(rng.choice([1, 1, 1, 2, 2, 3])):
        if rng.random() < 0.25:
            pieces.append(rng.choice(["x", " ", "-", "%%", "é", ": ", "\n"]))
        conv = rng.choice(RF_CONVS + "dxfegs")
        pieces.append(rf_spec(mode, args, conv))
    fmt = "".join(pieces)
    if mode == "hash":
        args = ["h:" + ",".join(k.encode().hex() + "=" + v for k, v in args[0].items())]
        if rng.random() < 0.05:
            args.append("i:1")
    elif rng.random() < 0.06 and args:
        args.pop()
    return " ".join([fmt.encode().hex() or "-"] + args)


family("rubyformat", "rubyformat", [rubyformat_line() for _ in range(int(os.environ.get("RUBYFORMAT_LINES", "12000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubyformat/oracle.rb")])


# --- Ruby's json 2.1, against Ruby 2.6 ----------------------------------------------------------------------------

RJ_STR_PIECES = [b"a", b"key", b" ", b"\"", b"\\", b"/", b"\n", b"\t", b"\r", b"\x08", b"\x0c", b"\x01", b"\x1f", b"\x7f", "é".encode(), "€".encode(),
                 "😀".encode(), "\u2028".encode(), b"\xff", b"\xc3", b"\x80", b"\xed\xa0\x80", b"\xf4\x90\x80\x80", b"\xc0\xaf", b"\xe0\x80\x80", b"\xf8\x88\x80\x80\x80"]


def rj_bytes(valid_only):
    pieces = RJ_STR_PIECES[:-8] if valid_only or rng.random() < 0.85 else RJ_STR_PIECES
    return b"".join(rng.choice(pieces) for _ in range(rng.randrange(0, 5)))


def rj_float():
    r = rng.random()
    if r < 0.3:
        return struct.unpack("<d", struct.pack("<Q", rng.randrange(1 << 64)))[0]
    if r < 0.8:
        return rng.choice([0.0, -0.0, 1.0, -1.5, 0.1, 1e15, 1e16, 1e17, 1e20, 1e-4, 1e-5, 0.0001234, 123456789.125, 1e100, 5e-324, 1.7976931348623157e308,
                           2.5, 100.0, 1e22, 1e23, 9007199254740993.0, 0.30000000000000004, 1234567890123456.7, 12345678901234567.0])
    return rng.choice([float("nan"), float("inf"), float("-inf")])


def rj_tree(depth, valid_only=False):
    r = rng.random()
    if depth > 3 or r < 0.45:
        k = rng.random()
        if k < 0.15:
            return rng.choice(["n", "t", "f"])
        if k < 0.4:
            return "i%d;" % rng.choice([0, -1, 7, 2 ** 62, -2 ** 63, 10 ** 30, rng.randrange(-10 ** 6, 10 ** 6)])
        if k < 0.6:
            x = rj_float()
            if valid_only and (x != x or x in (float("inf"), float("-inf"))):
                x = 1.5
            return "d" + struct.pack(">d", x).hex()
        return "s" + rj_bytes(valid_only).hex() + ";"
    n = rng.choice([0, 1, 2, 3, 4])
    if r < 0.72:
        return "a%d:" % n + "".join(rj_tree(depth + 1, valid_only) for _ in range(n))
    keys = [rng.choice([b"a", b"b", b"", b"key", "é".encode(), b"\"q\"", b"a"]) if rng.random() < 0.8 or valid_only else rj_bytes(False) for _ in range(n)]
    return "o%d:" % n + "".join("s" + k.hex() + ";" + rj_tree(depth + 1, valid_only) for k in keys)


def rj_deep(n, leaf):
    return "a1:" * (n - 1) + leaf if rng.random() < 0.5 else "o1:s61;" * (n - 1) + leaf


def rj_text_of(tree, i=0):
    """Python's rendering of a tree, with Ruby-compatible variety: spaces, comments, escapes."""
    c = tree[i]
    ws = lambda: rng.choice(["", "", "", " ", "\n  ", "\t", "/* c */", "// c\n", " /**/ "])
    if c in "ntf":
        return {"n": "null", "t": "true", "f": "false"}[c].encode(), i + 1
    if c == "i":
        j = tree.index(";", i)
        return tree[i + 1:j].encode(), j + 1
    if c == "d":
        x = struct.unpack(">d", bytes.fromhex(tree[i + 1:i + 17]))[0]
        t = repr(x) if x == x and x not in (float("inf"), float("-inf")) else "1.5"
        if rng.random() < 0.3 and "e" not in t and "." in t:
            t = t + "e" + rng.choice(["0", "+1", "-2", "10"])
        return t.replace("inf", "1e400").encode(), i + 17
    if c == "s":
        j = tree.index(";", i)
        raw = bytes.fromhex(tree[i + 1:j])
        out = b""
        for ch in raw.decode("utf-8", "replace"):
            k = rng.random()
            if ch in "\"\\":
                out += b"\\" + ch.encode()
            elif ord(ch) < 32:
                out += b"\\u%04x" % ord(ch)
            elif k < 0.15:
                cp = ord(ch)
                out += b"".join(b"\\u%04X" % u for u in ([cp] if cp < 0x10000 else [0xd800 + ((cp - 0x10000) >> 10), 0xdc00 + ((cp - 0x10000) & 0x3ff)]))
            elif k < 0.2 and ch == "/":
                out += b"\\/"
            else:
                out += ch.encode()
        return b'"' + out + b'"', j + 1
    j = tree.index(":", i)
    n = int(tree[i + 1:j])
    i = j + 1
    parts = []
    for _ in range(n):
        if c == "a":
            v, i = rj_text_of(tree, i)
            parts.append(ws().encode() + v + ws().encode())
        else:
            k, i = rj_text_of(tree, i)
            v, i = rj_text_of(tree, i)
            parts.append(ws().encode() + k + ws().encode() + b":" + ws().encode() + v + ws().encode())
    open_, close = (b"[", b"]") if c == "a" else (b"{", b"}")
    return open_ + b",".join(parts) + (ws().encode() if not parts else b"") + close, i


RJ_ODD = [b"", b" ", b"NaN", b"[NaN]", b"[1, Infinity]", b"-Infinity", b"[-Infinity]", b"[-Inf]", b"[Inf]", b"[Infinit]", b"01", b"-0", b"-0.0", b"1.e2", b".5",
          b"1.", b"1.5-", b"1.5e", b"1e5x", b"1E+2", b"-", b"--1", b"[-]", b"\"\\x\"", b"\"\\ud800\"", b"\"\\ud800\\u0041\"", b"\"\\ud800abcdefg\"",
          b"\"\\udc00x\"", b"\"\\ud83d\\ude00\"", b"\"a\tb\"", b"\"\\u12\"", b"[1]/*c*/", b"// c\n[1]", b"// c", b"/* c", b"/x", b"[1/x]", b"[1,/*",
          b"{\"a\":1,\"a\":2,\"b\":3}", b"[1]x", b"{\"a\" 1}", b"[\"a\"\"b\"]", b"1e400", b"-1e400", b"2.5e-400", b"[]]", b"true false", b"{1:2}",
          b"'a'", b"nul", b"[nul]", b"{\"a\":nul}", b"[1,]", b"{\"a\":1,}", b"[1 2]", b"{\"a\":[1,]}", b"[{\"a\":}]", b"{\"a\":NaN}", b"[\"\\ud800\"]",
          b"{\"\\ud800\":1}", b"\"\xff\"", b"[\"\xc3\"]", b"[1,\n2  ,3] ", b"\"\x00\"", b"[1]\x00", b"{\"a\"/**/:/**/1/**/}", b"[ /* */ ]", b"{ }"]


def rubyjson_line():
    r = rng.random()
    if r < 0.3:
        return f"{rng.choice(['gen', 'pgen'])} {rj_tree(0)}"
    if r < 0.33:
        n = rng.choice([99, 100, 101, 102])
        return f"{rng.choice(['gen', 'pgen'])} {rj_deep(n, rng.choice(['a0:', 'o0:', 'i1;', 'dfff8000000000000']))}"
    if r < 0.38:
        text = rng.choice(RJ_ODD)
    elif r < 0.43:
        # Numbers with many digits, near the edges of the doubles.
        digits = "".join(rng.choice("0123456789") for _ in range(rng.randrange(1, 40)))
        text = (rng.choice(["", "-"]) + rng.choice(["0", "1", "9", "4"]) + "." + digits + rng.choice(["", "e" + str(rng.randrange(-330, 310)), "E+" + str(rng.randrange(0, 30)),
                                                                                                     "e-3" + str(rng.randrange(0, 30))])).encode()
        if rng.random() < 0.3:
            text = b"[" + text + b"]"
    elif r < 0.46:
        n = rng.choice([99, 100, 101, 102])
        text = b"[" * n + rng.choice([b"", b"1", b"{}"]) + b"]" * n if rng.random() < 0.5 else b'{"a":' * n + b"1" + b"}" * n
    else:
        text, _ = rj_text_of(rj_tree(0, valid_only=True))
        text = rng.choice([b"", b" ", b"\n"]) + text + rng.choice([b"", b" ", b"\n", b"/* end */"])
        if rng.random() < 0.35:
            cs = bytearray(text)
            for _ in range(rng.randrange(1, 3)):
                i = rng.randrange(len(cs) + 1)
                k = rng.random()
                if k < 0.35 and cs:
                    del cs[min(i, len(cs) - 1)]
                else:
                    cs[i:i] = rng.choice([b",", b"]", b"}", b"[", b"{", b"\"", b"\\", b":", b"1", b"-", b".", b"e", b"/", b"*", b" ", b"x", b"\x01", b"n"])
            text = bytes(cs)
    if text.startswith(b"Infinity"):
        text = b" " + text
    return f"parse {text.hex() or '-'}"


family("rubyjson", "rubyjson", [rubyjson_line() for _ in range(int(os.environ.get("RUBYJSON_LINES", "12000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubyjson/oracle.rb")])


# --- JavaScript's Date, against Node 25 with TZ=UTC ---------------------------------------------------------------

JD_MONTHS = ["Jan", "February", "MAR", "apr", "May", "Mayday", "June", "Jul", "Augu", "Sep", "Oct", "Nov", "Dec", "Ja", "Ma", "Junk", "Decembe"]
JD_WORDS = ["Mon", "Tuesday", "Wed,", "GMT", "UTC", "UT", "Z", "z", "EST", "EDT", "CST", "cdt", "MST", "mdt", "PST", "PDT", "AM", "pm", "am", "PM", "T", "t",
            "x", "foo", "gmtx", "utcz", "é", " ", " ", "﻿", "᠎", "　", "_", "[", "@"]


def jd_hex(text):
    return "".join("%04x" % u for u in jd_units(text)) or "-"


def jd_units(text):
    out = []
    for ch in text:
        c = ord(ch)
        out += [c] if c < 0x10000 else [0xd800 + ((c - 0x10000) >> 10), 0xdc00 + ((c - 0x10000) & 0x3ff)]
    return out


def jd_num(digits=None):
    n = digits or rng.choice([1, 1, 2, 2, 2, 3, 4, 4, 5, 6, 7, 9, 10, 12])
    t = "".join(rng.choice("0123456789") for _ in range(n))
    if rng.random() < 0.3:
        t = str(rng.choice([0, 1, 12, 13, 23, 24, 25, 29, 30, 31, 32, 49, 50, 59, 60, 99, 100, 999, 1000, 1970, 2000, 9999, 10000])).zfill(n if rng.random() < 0.3 else 1)
    return t


def jd_iso():
    ok = rng.random() < 0.5
    two = lambda hi: str(rng.randrange(0, hi)).zfill(2)
    y = str(rng.randrange(0, 10000)).zfill(4) if ok else rng.choice([jd_num(4), "+" + jd_num(6), "-" + jd_num(6), "-000000", "+000000", jd_num(4), jd_num(rng.choice([3, 5]))])
    out = y
    if rng.random() < 0.8:
        out += "-" + (str(rng.randrange(1, 13)).zfill(2) if ok else rng.choice(["01", "12", "00", "13", jd_num(2), "1"]))
        if rng.random() < 0.8:
            out += "-" + (str(rng.randrange(1, 29)).zfill(2) if ok else rng.choice(["01", "31", "00", "32", jd_num(2), "1"]))
    if rng.random() < 0.7:
        if ok:
            out += "T" + rng.choice([two(24), "24"]) + ":" + two(60)
        else:
            out += rng.choice(["T", "T", "t", " ", "Tx"]) + rng.choice(["00", "23", "24", jd_num(2), "1"]) + rng.choice([":", ":", ""]) + rng.choice(["00", "59", "60", jd_num(2)])
        if rng.random() < 0.7:
            out += ":" + (two(60) if ok else rng.choice(["00", "59", "60", jd_num(2)]))
            if rng.random() < 0.6:
                out += rng.choice([".", "."]) + jd_num(rng.choice([1, 2, 3, 4, 6, 9, 12]))
        out += rng.choice(["", "", "Z", "z", "+00:00", "-05:30", "+0530", "+24:00", "-23:59", "+5", "+05", "+05:", "+05:3", " GMT", "Z ", "+0560"])
    return out


def jd_legacy():
    pieces = []
    for _ in range(rng.randrange(1, 8)):
        k = rng.random()
        if k < 0.35:
            pieces.append(jd_num())
        elif k < 0.5:
            pieces.append(rng.choice(JD_MONTHS))
        elif k < 0.65:
            pieces.append(rng.choice(JD_WORDS))
        else:
            pieces.append(rng.choice([" ", " ", "  ", "-", "/", ":", "::", ".", ",", "+", "-", ")", "(", "(x(y)z)", "(GMT)", "\t", "\n", "\u0000"]))
    sep = rng.choice(["", " ", " ", ""])
    return sep.join(pieces)


def jd_template():
    plausible = rng.random() < 0.65
    y = str(rng.randrange(1890, 2110)) if plausible else rng.choice([jd_num(4), jd_num(2), jd_num(1), jd_num(3)])
    mon = rng.choice(JD_MONTHS[:13]) if plausible else rng.choice(JD_MONTHS)
    d = str(rng.randrange(1, 29)).zfill(rng.choice([1, 2])) if plausible else rng.choice([jd_num(2), jd_num(1)])
    hh = (lambda: str(rng.randrange(0, 24)).zfill(2)) if plausible else (lambda: jd_num(2))
    mm = (lambda: str(rng.randrange(0, 60)).zfill(2)) if plausible else (lambda: jd_num(2))
    t = rng.choice(["", " " + hh() + ":" + mm(), " " + hh() + ":" + mm() + ":" + mm(), " " + str(rng.randrange(0, 14)) + ":" + mm() + " " + rng.choice(["AM", "PM", "am"]),
                    " " + hh() + ":" + mm() + ":" + mm() + "." + jd_num(3), " 12::"])
    z = rng.choice(["", "", " GMT", " UTC", " Z", " GMT+0530", " GMT-8", " +01:00", " EST", " (Coordinated Universal Time)", " GMT+12345", " UTC+5:", " -0000", " PST",
                    " pdt", " CST", " cdt", " EDT", " MST", " MDT", " UT", " z"])
    if rng.random() < 0.2:
        t = t.replace(".", "." + jd_num(rng.choice([4, 9, 10, 12, 15])) + "x")[:-1] if "." in t else t + ":" + mm() + "." + jd_num(rng.choice([1, 2, 10, 13]))
    out = rng.choice([f"{mon} {d}, {y}{t}{z}", f"{d} {mon} {y}{t}{z}", f"{y}/{d}/{d}{t}{z}", f"{d}/{d}/{y}{t}{z}", f"Tue, {d} {mon} {y}{t}{z}",
                       f"Sun {mon} {d} {y}{t}{z}", f"{mon} {d}{t}", f"{y}-{d}-{d}{t}{z}", f"{d}-{mon}-{y}{t}", f"{t.strip()} {d}/{d}/{y}"])
    if rng.random() < 0.15:
        out = out.replace(" ", rng.choice(["\ufeff", "\u00a0", "\u3000", "\u2028", "\u180e", "\t", "\n"]))
    if rng.random() < 0.1:
        out = out.replace(y, y + jd_num(rng.choice([6, 7]))) if len(y) == 4 else out
    return out


def jd_value():
    r = rng.random()
    if r < 0.4:
        return str(rng.randrange(-8640000000000000, 8640000000000001))
    if r < 0.6:
        return str(rng.choice([0, -1, 1, 8640000000000000, -8640000000000000, 8640000000000001, -8640000000000001, 951782400000, 253402300800000, 253402300799999,
                               -62198755200000, -62198755200001, -62167219200000, -62167219200001, 86399999, -86400000]))
    if r < 0.8:
        return repr(rng.uniform(-1e13, 1e13))
    return rng.choice(["nan", "inf", "-inf", "-0.0", "0.5", "-0.5", "1e+16", "8.64e+15", "-8.64e+15", "8640000000000000.5", "1.5", "-1.5"])


def jd_utc_arg():
    r = rng.random()
    if r < 0.6:
        return str(rng.randrange(-30, 3000))
    if r < 0.85:
        return rng.choice(["0", "99", "100", "-1", "-0.0", "12", "13", "-13", "1.9", "-1.9", "49.5", "99.9", "275760", "-271821", "1000000", "1000001", "-1000000",
                           "10000000", "-10000001", "1e+300", "3600000", "8.64e+15", "1e+9", "-400000", "-500000"])
    return rng.choice(["nan", "inf", "-inf", repr(rng.uniform(-1e8, 1e8))])


def jsdate_line():
    r = rng.random()
    if r < 0.6:
        k = rng.random()
        if k < 0.3:
            text = jd_iso() if rng.random() < 0.6 else "%04d-%02d-%02dT%02d:%02d:%02d.%03dZ" % (rng.randrange(0, 10000), rng.randrange(1, 13), rng.randrange(1, 32),
                                                                                                rng.randrange(0, 24), rng.randrange(0, 60), rng.randrange(0, 60), rng.randrange(0, 1000))
        elif k < 0.6:
            text = jd_template()
        elif k < 0.85:
            text = jd_legacy()
        else:
            text = rng.choice(["Thu, 01 Jan 1970 00:00:00 GMT", "Tue Nov 14 2023 22:13:20 GMT+0000 (Coordinated Universal Time)", "2023-11-14T22:13:20.123Z",
                               "Jan 15", "1/2/3", "12:00 AM 1/1/2000", "2000-01-01T24:00", "-000000-01-01", "+002000-01-01", "Jan 1 2000 GMT+1193047:",
                               "Jan 1 2000 GMT+400000:", "2000-01-01\u0000xyz", "-271821-04-20T00:00:00Z", "+275760-09-13T00:00:00.001Z", "Sat, 13 Sep 275760 00:00:00 GMT",
                               "Fri, 01 Jan -0001 00:00:00 GMT", "", " ", "x 2000", "x2000", "2000x", "(2000) 1 1", "1 (2 3) 4 5"])
        return "pa " + jd_hex(text)
    if r < 0.77:
        return "ut " + " ".join(jd_utc_arg() for _ in range(rng.randrange(0, 8)))
    if r < 0.8:
        # Years below -399999, where V8's day arithmetic divides negatives,
        # brought back into range by the day of the month.
        y = rng.randrange(-1000000, -399999)
        back = -round((y - 1970) * 365.2425)
        return f"ut {y} {rng.randrange(-30, 30)} {back + rng.randrange(-99000000, 99000000)} {rng.randrange(0, 24)}"
    return "fm " + jd_value()


family("jsdate", "jsdate", [jsdate_line() for _ in range(int(os.environ.get("JSDATE_LINES", "12000")))],
       ["env", "TZ=UTC", "node", os.path.join(LIB, "jsdate/oracle.js")])


# --- Node's legacy url.parse/format/resolve, against Node 25 ------------------------------------------------------

NU_SCHEMES = ["", "", "http:", "HTTP:", "https:", "ftp:", "file:", "FILE:", "javascript:", "JavaScript:", "mailto:", "x-y+z.1:", "ws:", "gopher:", "a:", "1:", "data:"]
NU_HOSTS = ["", "example.com", "EXAMPLE.com", "a..b", "x", "[::1]", "[::1", "[a:b]x", "a[b", "ex_ample.com", "a!$&()*+,=~b", "\x01ab", "a" * 260, "a" * 255, "a" * 256, "b" * 254 + ".c", "a\tb", "1.2.3.4",
            "h:", "h:80", "h:8a", "h::", "a b", "a%b", "a;b", "a<b", "a^b", "a|b", "a\x7fb", "\x00x", "Ex.Com.", "-a-", "a\\b", "a/b"]
NU_PATHS = ["", "/", "/a/b", "/a b", "/a\\b", "/a'b\"c", "/%7B{}", "a/b", "/é/😀", "/\ud800", "/a/../b", "/./c/", "//x", "///y", "/a;b", "/`^|"]
NU_QUERIES = ["", "", "?", "?a=1&b=2", "?a=1&a=2&c", "?x y", "?é=ü", "?%zz=1", "?a+b=c+d", "?#", "?a=%41&=&&b", "?q?r"]
NU_HASHES = ["", "", "#", "#frag", "#a b", "#a#b", "#?q", "#é"]


NU_FAST = ["///a'b", "/a'b", "//a'b", "///x?q", "//x/y'z?q r", "/ a", "/a?b c", "/a\tb", "///", "/^`{}", "//^", "///|?|", "/a\\b'"]


def nu_url():
    if rng.random() < 0.06:
        return rng.choice(NU_FAST)
    if rng.random() < 0.15:
        # ASCII only: a non-ASCII host name needs IDNA, which is outside the package.
        return "".join(rng.choice(["/", "\\", "?", "#", "@", ":", "a", "b", " ", "%", "h", ".", "[", "]", "x:", "//"]) for _ in range(rng.randrange(0, 10)))
    auth = rng.choice(["", "", "", "user@", "u:p@", "a@b@", "%41@", "%zz@", "%E9@", "é@", "a%40b@", "@", "a b@"])
    host = rng.choice(NU_HOSTS)
    port = rng.choice(["", "", "", ":80", ":", ":0123", ":x"])
    path = rng.choice(NU_PATHS)
    if not path.isascii() and host not in ("example.com", "x", "1.2.3.4", "EXAMPLE.com"):
        # Without a plain host before it, the path could become the host.
        path = "/a/b"
    out = (rng.choice(NU_SCHEMES) + rng.choice(["", "//", "//", "/", "///", "\\\\", "\\/"]) + auth + host + port + path + rng.choice(NU_QUERIES) +
           rng.choice(NU_HASHES))
    ws = rng.choice(["", "", "", " ", "\t", " ", "﻿", "\n", "\x00"])
    return rng.choice([ws + out, out + ws, ws + out + ws])


def nu_hex(text):
    return "".join("%04x" % u for u in jd_units(text)) or "-"


NU_RELS = ["", ".", "..", "../x", "./y", "z", "/abs", "//host/p", "?q", "#h", "mailto:x@y", "http:", "http:path", "https://other/p", "file:///etc", "../../../..",
           "a/b/../../..", "javascript:alert(1)", "g;x?y#s", "//u@h", "x:y", "http:/abs", "HTTP://H/", "..//", "a/./b/"]
NU_SOURCES = ["http://a/b/c/d;p?q", "mailto:local1@domain1", "http://u:p@h:1/x/y/", "file:///a/b", "/rel/path", "rel/path", "x:y/z", "http://h", "https://h/a/",
              "mailto:a@b", "news:comp.lang", "", "http://h/a/b/c/../", "//h/p", "ftp://f/x"]


def nu_format_field(kind):
    r = rng.random()
    if r < 0.35:
        return "u"
    if r < 0.42:
        return "n"
    if kind == "slashes":
        return rng.choice(["t", "f", "t", "n", "u"])
    if kind == "query":
        if rng.random() < 0.5:
            return "o:" + ",".join(nu_hex(rng.choice(["a", "b", "x y", "é"])) + "=" + nu_hex(rng.choice(["1", "", "a&b", "ü"])) for _ in range(rng.randrange(0, 3)))
        return nu_hex(rng.choice(["a=1", "", "x"]))
    choices = {"protocol": ["http", "http:", "https:", "file", "file:", "mailto:", "x", "javascript:", ""], "auth": ["u", "u:p", "a@b", "é", "a b", "\ud800", "\ud800x", ""],
               "host": ["h", "h:80", "", "[::1]"], "port": ["80", "", "0"], "hostname": ["h", "::1", "[::1]", "a:b", ""], "hash": ["#x", "x", ""],
               "search": ["?a", "a", "", "?a#b"], "pathname": ["/p", "p", "", "/a?b#c", "a#"]}[kind]
    return nu_hex(rng.choice(choices))


def nodelegacyurl_line():
    r = rng.random()
    if r < 0.6:
        text = nu_url()
        pq = rng.choice("01") if "\ud800" not in text else "0"
        return f"pa {nu_hex(text)} {pq} {rng.choice('001')}"
    if r < 0.85:
        src = rng.choice(NU_SOURCES) if rng.random() < 0.6 else nu_url()
        rel = rng.choice(NU_RELS) if rng.random() < 0.6 else nu_url()
        return f"rs {nu_hex(src)} {nu_hex(rel)}"
    return "fo " + " ".join(nu_format_field(k) for k in ["protocol", "slashes", "auth", "host", "port", "hostname", "hash", "search", "query", "pathname"])


family("nodelegacyurl", "nodelegacyurl", [nodelegacyurl_line() for _ in range(int(os.environ.get("NODELEGACYURL_LINES", "12000")))],
       ["node", "--no-deprecation", os.path.join(LIB, "nodelegacyurl/oracle.js")])


# --- Ruby's URI, against Ruby 2.6 ---------------------------------------------------------------------------------

RU_SCHEMES = ["http:", "HTTP:", "https:", "ftp:", "FTP:", "file:", "ldap:", "ldaps:", "mailto:", "MailTo:", "x-y+z.1:", "urn:", "a:", "1a:", "", "", ""]
RU_HOSTS = ["h", "Example.COM", "1.2.3.4", "1.2.3.4.5", "256.1.1.1", "[::1]", "[::1", "[1:2:3:4:5:6:7:8]", "[::ffff:1.2.3.4]", "[1::2:3]", "[v1.a]", "v1.a]",
            "[v1.:x]", "[fe80::1%25en0]", "", "a%41b", "a_b~c", "a b", "h:", "localhost", "[1:2:3:4:5:6:7::]", "[::1:2:3:4:5:6:7]", "[1:2:3:4:5:6:1.2.3.4]",
            "[::1.2.3.4]", "[1::1.2.3.4]", "[g::1]", "[::1]x", "[a:b:c:d:e:f:a:b:c]", "[::]", "[::1:2:3:4:5:6]", "[1::2:3:4:5:6]", "[::a:b:c:d:e:f]",
            "[::1:2:3:4:5:6:7]", "[1:2::3:4:5:6]", "[1:2:3::4:5:6]", "[::1:2]", "[::1:2:3]"]
RU_PATHS = ["", "/", "/a/b", "/a b", "/%41%zz", "a/b", "//x", "/a;type=a", "/;type=i", "/%2Fx", "//%2Fy", "/a/../b/./c", "/:@!$&'()*+,;=", "/[x]",
            "/dc=x,dc=y", "a@b", "a@b,c@d", "a@b;c@d,", "@b", "a@", "/a?b"]
RU_QUERIES = ["", "", "?", "?a=1&b=2", "?x y", "?\"<>'`", "?a\tb", "?cn,sn?sub?(o=x)?ext", "??", "?subject=hi&&body=x&", "?%zz", "?é"]
RU_FRAGS = ["", "", "#", "#f", "#a/b?c", "#x y", "#%41", "#a#b", "#[x]"]


def ru_uri():
    r = rng.random()
    if r < 0.12:
        return "".join(rng.choice(["/", "?", "#", "@", ":", "a", "b", " ", "%", "h", ".", "[", "]", "1", "v", "//", "%41", ";", "é"]) for _ in range(rng.randrange(0, 10)))
    if r < 0.2:
        return rng.choice(["//[::1]/x", "//[::1", "//[::1:2/x", "//[::1:2", "//[::1:2:3:4:5:6", "//[1::2:3", "//[::1.2.3.4:5", "//u@[::1:2:3/p",
                           "http://[::1:2:3:4:5:6]/", "//[::ffff:1.2.3.4", "//[1::2:3:4]:80/p", "//v1.a]/p", "//u@[::1:2", "//[::1.2.3.4", "//[1:2:3:4:5:6:7:8", "//[::/x", "//[]",
                           "mailto:", "mailto://h/p", "mailto:?subject=x", "ldap:x", "ldap://h/x#f", "ftp:foo", "file://localhost/x", "file:/x",
                           "http://user@host@x", "http://h:x/", "http://h:0080/", "http://h:99999999999999999999/", "http://[v1.x]/", "http://a b"])
    ok = rng.random() < 0.6
    auth = rng.choice(["", "", "", "user@", "u:p@", "@", ":@", "u:@", "a%41@"] + ([] if ok else ["a b@", "u@v@"]))
    port = rng.choice(["", "", "", ":80", ":", ":0443", ":21", ":389", ":8080"] + ([] if ok else [":x"]))
    hosts = ["h", "Example.COM", "1.2.3.4", "1.2.3.4.5", "[::1]", "[1::2:3]", "[v1.a]", "", "a%41b", "localhost", "[::1.2.3.4]"] if ok else RU_HOSTS
    paths = ["", "/", "/a/b", "a/b", "/a;type=a", "/;type=i", "/%2Fx", "//%2Fy", "/a/../b/./c", "/:@!$&'()*+,;=", "/dc=x,dc=y", "a@b", "a@b,c@d"] if ok else RU_PATHS
    frags = ["", "", "#", "#f", "#a/b?c", "#%41"] if ok else RU_FRAGS
    hier = rng.choice(["//" + auth + rng.choice(hosts) + port, "//" + auth + rng.choice(hosts) + port, ""])
    return rng.choice(RU_SCHEMES) + hier + rng.choice(paths) + rng.choice(RU_QUERIES) + rng.choice(frags)


RU_REFS = ["", "g", "./g", "g/", "/g", "//g", "?y", "g?y", "#s", "g#s", "g?y#s", ";x", "g;x", ".", "./", "..", "../", "../g", "../..", "../../", "../../g",
           "../../../g", "/./g", "/../g", "g.", ".g", "g..", "..g", "./../g", "./g/.", "g/./h", "g/../h", "http:g", "//u@h2:0080", "//:9", "mailto:x@y", "a:b",
           "ftp://f/x", "file:///z", "?", "#", "%zz", "[x]"]


def ru_hex(t):
    return t.encode().hex() or "-"


def rubyuri_line():
    r = rng.random()
    if r < 0.5:
        return "pa " + ru_hex(ru_uri())
    if r < 0.75:
        base = rng.choice(["http://a/b/c/d;p?q", "http://u:p@h:81/x/y/", "ftp://h/a/b", "file:///a/b", "mailto:a@b", "ldap://h/dc=x?cn", "a/b", "//h/p", "https://h",
                           "x:y/z", "http://h/a/..", "http://h/../a"]) if rng.random() < 0.6 else ru_uri()
        refs = [rng.choice(RU_REFS) if rng.random() < 0.7 else ru_uri() for _ in range(rng.choice([1, 1, 1, 2, 3]))]
        return "jo " + " ".join(ru_hex(x) for x in [base] + refs)
    if r < 0.82:
        data = bytes(rng.choice([0x20, 0x2a, 0x2d, 0x2e, 0x5f, 0x41, 0x7a, 0x30, 0x2b, 0x25, 0x26, 0x3d, 0x7e, 0x00, 0xff, 0xc3, 0xa9]) for _ in range(rng.randrange(0, 8)))
        return "ec " + (data.hex() or "-")
    if r < 0.9:
        t = "".join(rng.choice(["a", "+", "%41", "%e9", "%C3%A9", "%", "%4", "%zz", "%ff", " ", "=", "&", "é", "%2B"]) for _ in range(rng.randrange(0, 6)))
        return "dc " + ru_hex(t)
    if r < 0.95:
        items = []
        for _ in range(rng.randrange(0, 4)):
            k = ru_hex(rng.choice(["a", "b c", "é", "", "x&y", "1"]))
            v = rng.choice(["n", "s" + ru_hex(rng.choice(["1", "", "a b", "é", "=&"])), "a" + ".".join(rng.choice(["n", ru_hex("v"), ru_hex(""), ru_hex("w x")]) for _ in range(rng.randrange(0, 3)))])
            items.append(k + "=" + v)
        return "ew " + " ".join(items) if items else "ew"
    t = "".join(rng.choice(["a", "=", "&", "+", "%41", "%e9", "%C3%A9", "%ff", "%", "b", "&&", "==", "é"]) for _ in range(rng.randrange(0, 7)))
    return "dw " + ru_hex(t)


family("rubyuri", "rubyuri", [rubyuri_line() for _ in range(int(os.environ.get("RUBYURI_LINES", "12000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubyuri/oracle.rb")])


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
