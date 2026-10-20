"""B58's packages against the references they reproduce.

- Go 1.25's go/build/constraint: Parse of //go:build and // +build lines,
  Expr.String, Eval (with the order it asks about tags), PlusBuildLines,
  GoVersion, IsGoBuild and IsPlusBuild, syntax errors with their offsets.
- Go 1.25's go/version: Compare, IsValid and Lang.
- Go 1.25's strings: Fields, EqualFold, ToUpper/ToLower/ToTitle, Title,
  ToValidUTF8, Split(After)(N), Cut(Prefix/Suffix), Count, (Last)Index,
  IndexRune, (Last)IndexAny, Replace, NewReplacer, Trim*, and unicode's
  ToUpper, ToLower, ToTitle and SimpleFold over all runes.
- Go 1.25's net/mail: ParseAddress, ParseAddressList, Address.String and
  ParseDate (run with TZ=UTC), error texts included.
- Java 17's java.net.URLEncoder.encode and URLDecoder.decode with UTF-8,
  exception messages included.
- Java 17's DateTimeFormatter.ofPattern(pattern, Locale.US) formatting
  LocalDate, LocalTime and LocalDateTime, exceptions included.
- Go 1.25's html.EscapeString and html.UnescapeString.
- Go 1.25's net/http ParseCookie, ParseSetCookie, Cookie.String and
  Cookie.Valid (run with TZ=UTC), errors included.
- Java 17's java.text.SimpleDateFormat(pattern, Locale.US) in GMT,
  formatting instants from 1584 on, pattern errors included.

Environment: B58_ROOT, B58_PKG, B58_LIB, SEED; <FAMILY>_LINES sets a
family's size; B58_ONLY=a,b runs only those families; B58_SPREAD=<family>
prints how that family's oracle answers are spread.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B58_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B58_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B58_LIB") or os.path.join(ROOT, "scripts/lib/b58")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b58")
harness = os.path.join(work, "b58test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b58test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stdout[-800:] + built.stderr[-1500:]); sys.exit(1)

cases = []   # (family, op, path, want lines)
hx = lambda t: t.encode("utf-8").hex() or "-"
hb = lambda b: b.hex() or "-"


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
    only = os.environ.get("B58_ONLY")
    if only and name not in only.split(","):
        return
    want = subprocess.run(oracle_argv, input="\n".join(lines) + "\n", capture_output=True, text=True).stdout.splitlines()
    if len(want) != len(lines):
        print(f"{name}: the oracle answered {len(want)} of {len(lines)}"); sys.exit(1)
    if os.environ.get("B58_SPREAD") == name:
        spread(want)
    for k in range(0, len(lines), chunk):
        cases.append((name, op, case_file("\n".join(lines[k:k + chunk]) + "\n"), want[k:k + chunk]))


def spread(want):
    """How the oracle's answers are spread: by first field, and the commonest error texts."""
    from collections import Counter
    def text(f):
        try:
            return bytes.fromhex(f).decode("utf-8", "replace")[:70]
        except ValueError:
            return f[:70]
    first = Counter(w.split(" ")[0] for w in want)
    errs = Counter(text(w.split(" ")[-1]) for w in want if w.split(" ")[0] in ("E", "X", "ERR"))
    print(f"spread: {len(want)} answers, {len(set(want))} distinct; first fields {dict(first.most_common(8))}")
    for e, n in errs.most_common(25):
        print(f"  {n:6d}  {e}")


def lines_for(name, default):
    return int(os.environ.get(name.upper() + "_LINES", str(default)))


# --- Go's go/build/constraint, against Go 1.25 -------------------------------------------------------------

BC_TAGS = ["linux", "darwin", "amd64", "arm64", "cgo", "go1", "go1.21", "go1.5", "go1.0", "go1.007", "go1.22", "xgo1.5", "go1.2.3",
           "go1.", "go1x", "go1.9223372036854775807", "go1.9223372036854775808",
           "go1.18446744073709551621", "go1.99999999999999999999", "go1.18_x", "ignore", "a", "b", "c", "_x", ".", "a.b",
           "é", "ǅ", "٣", "ℵ", "日本", "𝔸", "x١", "go1.٣", "Go1.4", "go12", "ª", "µ", "ﬁ", "ⅷ", "x²", "x\u0301", "a" * 30]
BC_BAD = ["-", "$", "#", "\u00a0", "\v", "\r", "\u2003", "ⅷ", "²", "\u0301", "@", "/", "=", ",", "&", "|", "&&&", "|||", "!!", "()", ")", "("]
BC_RAW = [b"\xff", b"\xc3", b"\x80", b"\xed\xa0\x80", b"\xe2\x80", b"\xf4\x90\x80\x80", b"\xc0\xaf", b"\xf0\x9f\x98"]


def bc_sp():
    r = rng.random()
    if r < 0.75:
        return " "
    if r < 0.92:
        return ""
    return rng.choice(["  ", "\t", " \t", "\u00a0", "\v", "\u2003"])


def bc_tree(depth):
    r = rng.random()
    if depth <= 0 or r < 0.3:
        return rng.choice(BC_TAGS[:20] if rng.random() < 0.7 else BC_TAGS)
    if r < 0.45:
        return "!" + bc_sp() * (rng.random() < 0.2) + bc_wrap(bc_tree(depth - 1))
    op = rng.choice(["&&", "||"])
    return bc_wrap(bc_tree(depth - 1)) + bc_sp() + op + bc_sp() + bc_wrap(bc_tree(depth - 1))


def bc_wrap(s):
    return "(" + bc_sp() * (rng.random() < 0.3) + s + bc_sp() * (rng.random() < 0.3) + ")" if rng.random() < 0.3 else s


def bc_mutate(s):
    for _ in range(rng.randrange(1, 3)):
        i = rng.randrange(0, len(s) + 1)
        k = rng.random()
        if k < 0.35 and s:
            s = s[:i] + s[i + 1:]
        elif k < 0.8:
            s = s[:i] + rng.choice(BC_BAD + ["!", "(", ")", "&&", "||", " ", "x"]) + s[i:]
        else:
            s = s[:i] + rng.choice(BC_TAGS) + s[i:]
    return s


def bc_big():
    n = rng.choice([998, 999, 1000, 1001, 1002, 500])
    op = rng.choice([" || ", " && ", " || !"])
    return op.join(rng.choice(["a", "b", "go1.3"]) for _ in range(n))


def bc_gobuild():
    r = rng.random()
    if r < 0.03:
        expr = bc_big()
    else:
        expr = bc_tree(rng.randrange(0, 5))
        if rng.random() < 0.3:
            expr = bc_mutate(expr)
        if rng.random() < 0.08:
            expr = expr[:rng.randrange(0, len(expr) + 1)]
    pre = rng.choice(["//go:build "] * 40 + ["//go:build\t", "//go:build", "//go:buildx ", " //go:build ", "// go:build ", "//go:build  ",
                                             "//go:build\u00a0", "//go:build\u2003", "//go:Build ", "/go:build "])
    tail = rng.choice([""] * 50 + [" ", "\n", "\t", "\r", "\u0085", "\u00a0", "\n\n", "\ny", " \n"])
    return pre + expr + tail


def bc_word():
    r = rng.random()
    if r < 0.6:
        w = rng.choice(BC_TAGS)
    elif r < 0.8:
        w = rng.choice(BC_TAGS) + rng.choice(BC_BAD) + rng.choice(["", "x"])
    else:
        w = rng.choice(["", "!", "!!", "!!x", "!x", "!" + rng.choice(BC_TAGS), "!-", "!é"])
    if rng.random() < 0.25 and not w.startswith("!"):
        w = "!" + w
    return w


def bc_plus():
    if rng.random() < 0.04:
        # One clause: its ANDs alone reach the limit.
        body = ",".join(rng.choice(BC_TAGS[:20] + ["!a", "!!a", "!", "x-y"]) for _ in range(rng.choice([100, 101, 102, 103])))
    elif rng.random() < 0.06:
        # Near the limit of 100 operators, counting ANDs and ORs together.
        target = rng.choice([98, 99, 100, 101, 102, 103])
        clauses, ops = [], 0
        while True:
            k = rng.choice([1, 1, 2, 3, 5, 20, 60])
            need = k - 1 + (1 if clauses else 0)
            if ops + need > target:
                break
            clauses.append(",".join(rng.choice(BC_TAGS[:20] + ["!a", "!!a", "!", "x-y"]) for _ in range(k)))
            ops += need
        while ops < target:
            clauses[-1] += ",a"
            ops += 1
        body = " ".join(clauses)
    else:
        clauses = []
        for _ in range(rng.randrange(0, 5)):
            clauses.append(",".join(bc_word() for _ in range(rng.randrange(1, 4))))
        body = "".join(c + rng.choice([" ", " ", "  ", "\t", "\u2003", "\u3000", "\u0085", "\v"]) for c in clauses).rstrip(" ")
    pre = rng.choice(["// +build "] * 40 + ["//+build ", "//   +build\t", "// +buildx ", "// +build", "//\t+build ", " // +build ",
                                             "// + build ", "//\u00a0+build ", "/ +build ", "// +BUILD "])
    tail = rng.choice([""] * 50 + [" ", "\n", "\t", "\u0085", "\n\n", "\nx"])
    return pre + body + tail


def bc_line():
    r = rng.random()
    if r < 0.6:
        s = bc_gobuild()
    elif r < 0.95:
        s = bc_plus()
    else:
        s = rng.choice(["", "//", "// ", "package main", "//go:", "//+", "/* +build x */", "//go:generate x", "// +buil d"])
    b = s.encode()
    if rng.random() < 0.05:
        i = rng.randrange(0, len(b) + 1)
        b = b[:i] + rng.choice(BC_RAW) + b[i:]
    tags = [t for t in BC_TAGS if rng.random() < 0.3]
    tl = ",".join(hx(t) for t in tags) if tags else "~"
    return f"p {hb(b)} {tl}"


family("gobuildconstraint", "gobuildconstraint", [bc_line() for _ in range(lines_for("gobuildconstraint", 12000))],
       [go_tool("gobuildconstraint", os.path.join(LIB, "gobuildconstraint/oracle.go"))])


# --- Go's go/version, against Go 1.25 -------------------------------------------------------------------------

GV_NUMS = ["0", "1", "2", "9", "10", "20", "21", "22", "99", "100", "01", "00", "007", "123456789012345678901234567890", "18446744073709551616"]


def gv_num():
    return rng.choice(GV_NUMS[:9] if rng.random() < 0.7 else GV_NUMS)


def gv_version():
    r = rng.random()
    if r < 0.03:
        return rng.choice(["", "go", "g", "go.", "go-1", "-go1", "1.21", "Go1.21", "go 1.21", "go1.21 ", "go1..1", "go1.21.", "go1.2.3.4",
                           "go1.21rc", "go1.21RC1", "go1.21rc1.2", "go1.21rc01", "go1.21rc0", "go1.21.0rc1", "gox", "go1-", "go1--x"])
    v = rng.choice(["go"] * 20 + ["", "Go", "g", "gox", "GO"]) + ("1" if rng.random() < 0.5 else gv_num())
    shape = rng.random()
    if shape < 0.15:
        pass
    elif shape < 0.4:
        v += "." + gv_num()
    elif shape < 0.75:
        v += "." + gv_num() + "." + gv_num()
    else:
        v += "." + gv_num() + rng.choice(["rc", "beta", "alpha", "rc", "beta", "a", "z", "foo", "rcx", "RC", "r-c", "rc.", "ß"])
        if rng.random() < 0.8:
            v += gv_num()
    if rng.random() < 0.1:
        v += rng.choice(["-bigcorp", "-", "-x-y", ".", "x", " ", "-go1.99", "\u00e9".encode().decode("unicode_escape")])
    return v


def gv_release(base):
    r = rng.random()
    if r < 0.25:
        return base
    if r < 0.5:
        return base + "." + gv_num()
    return base + rng.choice(["rc", "beta", "alpha", "a", "zz"]) + (gv_num() if rng.random() < 0.8 else "")


def goversion_line():
    if rng.random() < 0.25:
        base = "go1." + rng.choice(["19", "20", "21", "22", "100", "2", "0"])
        return f"{hx(gv_release(base))} {hx(gv_release(base))}"
    x = gv_version()
    y = gv_version() if rng.random() < 0.7 else x
    if rng.random() < 0.2:
        y = x.replace("rc", "beta") if "rc" in x else x + rng.choice([".0", ".1", "rc1", "-x"])
    return f"{hx(x)} {hx(y)}"


family("goversion", "goversion", [goversion_line() for _ in range(lines_for("goversion", 12000))],
       [go_tool("goversion", os.path.join(LIB, "goversion/oracle.go"))])


# --- Go's strings and unicode case mapping, against Go 1.25 ------------------------------------------------

GS_ASCII = ["a", "b", "A", "B", "k", "K", "s", "S", "i", "I", "x", "_", "1", " ", " ", "\t", "\n", "\v", "\f", "\r", ",", ".", "-", "=",
            "ab", "aa", "abc", "'", "Hello", "world", "go"]
GS_UNI = ["\u0085", " ", " ", " ", " ", "​", " ", " ", "　", " ", " ", "᠎", "̀",
          "ſ", "K", "Å", "å", "Å", "İ", "ı", "ß", "ẞ", "ǅ", "ǆ", "Ǆ", "ǈ", "ǉ", "Ǉ", "σ", "ς", "Σ", "ϑ", "θ", "Θ", "ϴ", "µ", "μ", "Μ",
          "ͅ", "ι", "Ι", "ι", "ᾳ", "ᾼ", "ﬓ", "Ⴀ", "ⴀ", "ა", "Ა", "Ꭰ", "ꭰ", "𐐀", "𐐨", "𞤀", "𞤢", "é", "É", "ё", "Ё", "́",
          "٣", "日", "😀", "�", "ȿ", "Ȿ", "ǰ", "ŉ", "ꙋ", "ᲀ", "ꮀ", "ｶ", "ａ", "Ａ", "ǯ", "Ʒ", "ʒ", "Ω", "Ω", "ω"]
GS_RAW = [b"\xff", b"\xc3", b"\x80", b"\xe2\x82", b"\xed\xa0\x80", b"\xf0\x9f\x98", b"\xf4\x90\x80\x80", b"\xc0\x80", b"\xfe", b"\xa9"]
GS_FOLD = [["k", "K", "K"], ["s", "S", "ſ"], ["å", "Å", "Å"], ["σ", "ς", "Σ"], ["θ", "ϑ", "Θ", "ϴ"], ["µ", "μ", "Μ"],
           ["ι", "Ι", "ͅ", "ι"], ["ǆ", "ǅ", "Ǆ"], ["ǉ", "ǈ", "Ǉ"], ["ß", "ẞ"], ["i", "I"], ["ⴀ", "Ⴀ"], ["ა", "Ა"], ["Ꭰ", "ꭰ"],
           ["𐐀", "𐐨"], ["é", "É"], ["ё", "Ё"], ["a", "A"], ["b", "B"], ["x", "X"], ["ȿ", "Ȿ"], ["Ω", "Ω", "ω"], ["ｶ"], ["ａ", "Ａ"],
           ["ꙋ", "ᲈ", "Ꙋ"], ["Ʒ", "ʒ"]]
GS_FOLD_OF = {c: g for g in GS_FOLD for c in g}


def gs_piece():
    r = rng.random()
    if r < 0.55:
        return rng.choice(GS_ASCII).encode()
    if r < 0.93:
        return rng.choice(GS_UNI).encode()
    return rng.choice(GS_RAW)


def gs_pieces(lo=0, hi=8):
    return [gs_piece() for _ in range(rng.randrange(lo, hi))]


def gs_text(lo=0, hi=8):
    return b"".join(gs_pieces(lo, hi))


def gs_sep(s):
    r = rng.random()
    if r < 0.15:
        return b""
    if r < 0.6 and s:
        i = rng.randrange(0, len(s))
        return s[i:i + rng.randrange(1, 4)]
    return rng.choice([b" ", b",", b"a", b"ab", b"\xc3", b"\xa9", "é".encode(), "　".encode(), b"aa", b"--"]) if rng.random() < 0.7 else gs_text(1, 3)


def gs_fold_variant(ps):
    out = []
    for p in ps:
        try:
            c = p.decode()
        except UnicodeDecodeError:
            out.append(p if rng.random() < 0.8 else rng.choice(GS_RAW))
            continue
        g = GS_FOLD_OF.get(c)
        out.append(rng.choice(g).encode() if g else p)
    if rng.random() < 0.15:
        i = rng.randrange(0, len(out) + 1)
        k = rng.random()
        if k < 0.4 and out:
            del out[min(i, len(out) - 1)]
        elif k < 0.7:
            out.insert(i, gs_piece())
        elif out:
            out[min(i, len(out) - 1)] = gs_piece()
    return b"".join(out)


GS_RUNE_BLOCKS = [(0, 0x250), (0x250, 0x370), (0x370, 0x400), (0x400, 0x530), (0x530, 0x590), (0x10a0, 0x1100), (0x13a0, 0x1400),
                  (0x1c80, 0x1cc0), (0x1e00, 0x2000), (0x2100, 0x2200), (0x2460, 0x24f0), (0x2c00, 0x2d30), (0xa640, 0xa800),
                  (0xab70, 0xabc0), (0xff00, 0xff60), (0x10400, 0x10500), (0x10570, 0x105c0), (0x10c80, 0x10d00), (0x118a0, 0x11900),
                  (0x16e40, 0x16e80), (0x1e900, 0x1e950), (0xd7f0, 0xe010)]


GS_ORBIT_RUNES = [0x212a, 0x212b, 0x17f, 0x1e9e, 0xdf, 0x130, 0x131, 0x3c2, 0x3d0, 0x3d1, 0x3d5, 0x3d6, 0x3f0, 0x3f1, 0x3f4, 0x3f5, 0x1e9b,
                  0x1fbe, 0x2126, 0x345, 0x3b9, 0x399, 0x3c9, 0x3a9, 0x1c4, 0x1c5, 0x1c6, 0x1c7, 0x1c8, 0x1c9, 0x1ca, 0x1cb, 0x1cc, 0x1f1,
                  0x1f2, 0x1f3, 0x6b, 0x4b, 0x73, 0x53, 0xb5, 0x39c, 0x3bc, 0xe5, 0xc5, 0x1c80, 0x1c81, 0x1c82, 0x1c83, 0x1c84, 0x1c85,
                  0x1c86, 0x1c87, 0x1c88, 0xa64a, 0xa64b, 0x1e60, 0x1e61, 0x398, 0x3b8, 0x3a3, 0x3c3, 0x392, 0x3b2, 0x395, 0x3b5]


def gs_rune():
    r = rng.random()
    if r < 0.15:
        return rng.choice(GS_ORBIT_RUNES)
    if r < 0.75:
        lo, hi = rng.choice(GS_RUNE_BLOCKS)
        return rng.randrange(lo, hi)
    if r < 0.95:
        return rng.randrange(0, 0x110000)
    return rng.choice([-1, -100, 0x110000, 0x7fffffff, -2147483648, 0x10ffff, 0xfffd, 0xd800, 0xdfff])


def gostrings_line():
    r = rng.random()
    if r < 0.2:
        return f"K {gs_rune()}"
    s = gs_text()
    if r < 0.27:
        return f"F {hb(s)}"
    if r < 0.37:
        ps = gs_pieces(0, 8)
        a = b"".join(ps)
        return f"E {hb(a)} {hb(gs_fold_variant(ps) if rng.random() < 0.85 else gs_text())}"
    if r < 0.45:
        return f"U {hb(s)}"
    if r < 0.5:
        return f"V {hb(s)} {hb(rng.choice([b'', b'?', '�'.encode(), b'ab', b'\xff']))}"
    if r < 0.58:
        return f"S {hb(s)} {hb(gs_sep(s))} {rng.choice([-2, -1, 0, 1, 2, 3, 5, 100])}"
    if r < 0.66:
        return f"C {hb(s)} {hb(gs_sep(s))}"
    if r < 0.73:
        s = gs_text(0, 14)
        chars = rng.choice([b"", b"a", b"ab", b"\xe9", "é".encode(), "�".encode(), "a�".encode(), b"\xff\xfe", b" \t,.", b"xyz=",
                            "　k".encode(), b"\xc3", gs_text(1, 3), gs_text(1, 3)])
        return f"A {hb(s)} {hb(chars)}"
    if r < 0.78:
        rr = rng.choice([97, 32, 0, 65533, -1, 0x110000, 0xd800, ord("é"), ord("　"), ord("😀"), ord("K"), 127, 128, 255])
        if rng.random() < 0.3:
            ps = [p for p in [gs_piece() for _ in range(3)]]
            try:
                rr = ord(ps[0].decode()[0])
            except (UnicodeDecodeError, IndexError):
                pass
        if rr in (-1, 0x110000, 0xd800) and rng.random() < 0.5:
            s += "\ufffd".encode()
        return f"I {hb(s)} {rr}"
    if r < 0.85:
        old = gs_sep(s)
        new = rng.choice([b"", b"X", b"YY", "é".encode(), b"\xff", old])
        return f"R {hb(s)} {hb(old)} {hb(new)} {rng.choice([-1, -1, 0, 1, 2, 3, 10])}"
    if r < 0.93:
        kind = rng.random()
        pairs = []
        if kind < 0.2:
            pairs = [(rng.choice([b"ab", b"aa", b"abc", "é".encode(), b"\xc3\xa9", b"--"]), rng.choice([b"", b"X", b"YY", b"a"]))]
        elif kind < 0.4:
            for _ in range(rng.randrange(1, 5)):
                pairs.append((rng.choice([b"a", b"b", b"k", b" ", b"\xc3", b"\xa9", b"\xff"] + ([s[j:j + 1] for j in [rng.randrange(len(s))]] if s else [])), rng.choice([b"A", b"1", b"\xff", b"-"])))
        elif kind < 0.6:
            for _ in range(rng.randrange(1, 5)):
                pairs.append((rng.choice([b"a", b"b", b"k", b" ", b"\xc3", b"\xa9"]), rng.choice([b"", b"AA", b"1", "é".encode(), b"-"])))
        else:
            for _ in range(rng.randrange(1, 5)):
                pairs.append((rng.choice([b"", b"", b"a", b"aa", b"ab", b"b", "é".encode(), b"\xc3", b"\xa9", b"k", b"x", b" "]) if rng.random() < 0.5 else gs_sep(s),
                              rng.choice([b"", b"1", b"22", "é".encode(), b"-", b"\xff"])))
        return "P " + hb(s) + "".join(f" {hb(o)} {hb(n)}" for o, n in pairs)
    cut = rng.choice([b"", b" ", b"a", b"ab", b" \t\n", "　".encode(), "é".encode(), b"\xff", b"\xc3", "�".encode(), b"\xa9a", gs_text(1, 3)])
    if rng.random() < 0.3:
        s = cut + s + cut
    return f"T {hb(s)} {hb(cut)}"


family("gostrings", "gostrings", [gostrings_line() for _ in range(lines_for("gostrings", 20000))],
       [go_tool("gostrings", os.path.join(LIB, "gostrings/oracle.go"))])


# --- Go's net/mail, against Go 1.25 (TZ=UTC) ----------------------------------------------------------------

NM_ATOMS = ["John", "Doe", "J.", "Mary-Jane", "O'Neil", "a", "Z9", "Ünïcödé", "日本", "x!#$%&'*+-/=?^_`{|}~"]
NM_ATOMS_ODD = [".x", "a..b", " x"]
NM_QUOTED = ['"John Doe"', '"a\\"b"', '"a\\\\b"', '"tab\tx"', '""', '"é"', '"a,b"', '"\\ü"', '"x@y"', '"Doe, John"', '"a\\ b"', '"(c)"']
NM_QUOTED_ODD = ['"unclosed', '"bad\x01"', '"esc\\\x01"', '"\x7f"']
NM_WORDS = ["=?utf-8?q?J=C3=B6rg?=", "=?UTF-8?B?SsO2cmc=?=", "=?iso-8859-1?q?J=F6rg?=", "=?us-ascii?q?a=80b?=", "=?iſo-8859-1?q?x?=",
            "=?uſ-ascii?q?x?=", "=?utf-8?Q?a_b?=", "=?utf-8?q?=ff?=", "=?utf-8?q?=e2=82?=", "=?utf-8?b?4oKs?=", "=?utf-8?q?=e2=82=ac?=",
            "=?UTF-8?Q?=E6=97=A5?=", "=?utf-8?b?YW\r\nJj?="]
NM_WORDS_ODD = ["=?koi8-r?q?x?=", "=?KOI8-R?B?eA==?=", "=?ÜTF-8?q?x?=", "=?ĲX?q?x?=", "=?utf-8?x?abc?=", "=?utf-8?b?!!?=", "=?utf-8?q?=ZZ?=",
                "=?utf-8?q?a?b?=", "=?utf-8?q??=", "=??q?x?=", "=?utf-8?b?YQ?=", "=?utf-8?q?a=2?="]
NM_LOCALS = ["a", "john.doe", "a.b.c", "a+tag", "ü", "日本", '"john doe"', '"a\\"b"', '"a.b"', "x!#$%&'*+-/=?^_`{|}~", '"x"', "first.last"]
NM_LOCALS_ODD = [".a", "a.", "a..b", '""', '"bad\x01"', "a\x00b"]
NM_DOMAINS = ["example.com", "x", "a.b", "[1.2.3.4]", "[::1]", "例え.jp", "[::ffff:1.2.3.4]", "mail.example.org"]
NM_DOMAINS_ODD = ["[IPv6:::1]", "[300.1.1.1]", "[fe80::1%eth0]", "[1.2.3.4 ]", "[1.2.3.4", "[a[b]", "[\\x]", ".com", "a..b", "com.", "[]",
                  "[01.2.3.4]", "[ü]", "[1.2.3.4]x"]
NM_COMMENTS = ["(comment)", "(nested (c))", "(a\\)b)", "(=?utf-8?q?x?= y)", "( )", "(J=?utf-8?q?=C3=B6?=)", "(\t=?utf-8?q?a?=\t=?utf-8?q?b?=\t)"]
NM_COMMENTS_ODD = ["(unclosed", "(=?koi8-r?q?x?=)", "(\\", "(a\\", "(\xff)"]


def nm_choice(ok, odd, p=0.85):
    return rng.choice(ok) if rng.random() < p else rng.choice(odd)


def nm_spec():
    return nm_choice(NM_LOCALS, NM_LOCALS_ODD) + ("@" if rng.random() < 0.92 else rng.choice([" @", "@ ", ""])) + nm_choice(NM_DOMAINS, NM_DOMAINS_ODD)


def nm_name():
    words = []
    for _ in range(rng.randrange(1, 4)):
        r = rng.random()
        words.append(nm_choice(NM_ATOMS, NM_ATOMS_ODD, 0.9) if r < 0.5 else nm_choice(NM_QUOTED, NM_QUOTED_ODD, 0.9) if r < 0.7 else nm_choice(NM_WORDS, NM_WORDS_ODD, 0.8))
    sep = rng.choice([" ", " ", " ", "  ", "\t", "", " (c) "])
    return sep.join(words)


def nm_address():
    r = rng.random()
    if r < 0.3:
        a = nm_spec()
        if rng.random() < 0.3:
            a += rng.choice([" ", ""]) + nm_choice(NM_COMMENTS, NM_COMMENTS_ODD)
        return a
    if r < 0.7:
        return nm_name() + rng.choice([" ", " ", ""]) + "<" + nm_spec() + (">" if rng.random() < 0.92 else rng.choice(["", "> "]))
    if r < 0.8:
        return "<" + nm_spec() + ">"
    if r < 0.92:
        members = [nm_address() if rng.random() < 0.3 else nm_spec() for _ in range(rng.randrange(0, 3))]
        return nm_name() + ":" + rng.choice([" ", ""]) + ", ".join(members) + rng.choice([";", ";", ";", "", "; (c)", ";x"])
    return rng.choice(["", " ", "@", "<>", "<@>", "a@", "@b", "a@b@c", "Undisclosed recipients:;", "a <b>", "(c) a@b", "a@b (c", "a@b, c@d",
                       "\"\"@x", "<a@b> x", "a@b;", "\xc3", "<a@b"])


def nm_noise(t):
    b = t.encode()
    if rng.random() < 0.07:
        i = rng.randrange(0, len(b) + 1)
        b = b[:i] + rng.choice([b"\xff", b"\xc3", b"\x01", b"\r\n", b",", b"<", b">", b"(", b")", b"\"", b"\\", b";", b":", b"@", b"\x7f"]) + b[i:]
    if rng.random() < 0.03 and b:
        i = rng.randrange(0, len(b))
        b = b[:i] + b[i + 1:]
    return b


NM_ZONES = ["+0000", "-0700", "+0530", "-0000", "+1400", "+9999", "GMT", "UT", "UTC", "EST", "PDT", "Z", "CEST", "MeST", "ChST", "GMT+3",
            "GMT-11", "XYZW", "+00", "-07:00", "est", "A"]


def nm_pick(ok, odd, p=0.95):
    return rng.choice(ok) if rng.random() < p else rng.choice(odd)


def nm_date():
    if rng.random() < 0.04:
        return rng.choice(["", " ", "Mon", "T", "TTTTTT", "1 Jan 2006 00:00 T", "-0700", "+++++", "x(y)", "\r", "\r\n"]).encode()
    dow = nm_pick(["", "", "Mon, ", "Tue, ", "Sun, ", "Fri, ", "Sat, ", "Wed, ", "Thu, "], ["Fri,", "mon, ", "Monday, ", "Thu "])
    day = nm_pick(["1", "01", "2", "15", "28", "29", "30", "31", "9", "10"], ["32", "0", "001", " 2"])
    mon = nm_pick(["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"], ["jan", "JAN", "Sept", "Foo"])
    year = nm_pick(["2006", "1999", "2024", "06", "69", "68", "00", "99", "0000", "9999", "1970", "2038"], ["123", "20066"])
    hm = nm_pick(["15:04", "00:00", "23:59", "12:00", "01:30"], ["24:00", "12:60", "1:04", "15:4"])
    sec = nm_pick([":05", ":05", "", ":59", ":00", ":05.123", ":05.999999999"], [":60", ":05.", ":5"])
    zone = nm_pick(["+0000", "-0700", "+0530", "-0000", "+1400", "GMT", "UT", "UTC", "EST", "PDT", "Z", "CEST", "MeST", "ChST", "GMT+3", "GMT-11"],
                   ["+9999", "XYZW", "+00", "-07:00", "est", "A"], 0.9)
    sp = nm_pick([" "], ["  ", "\r\n ", "\t", "\r"], 0.9)
    tail = rng.choice([""] * 30 + [" (PST)", " (nested (c))", " (unclosed", " garbage", "(x)", " ", "\r\n", " (a) (b)", "\xff", " (PST)", "(x)"])
    t = dow + day + " " + mon + " " + year + " " + hm + sec + sp + zone + tail
    if rng.random() < 0.05:
        t = rng.choice([" ", "\t", "  "]) + t
    b = t.encode()
    if rng.random() < 0.04:
        i = rng.randrange(0, len(b) + 1)
        b = b[:i] + rng.choice([b"\xff", b"-", b"+", b"T", b"(", b"\r"]) + b[i:]
    return b


def nm_string_case():
    r = rng.random()
    if r < 0.5:
        pieces = ["a", "B", " ", "\t", "é", "日本", "\x01", "\x7f", "\"", "\\", ".", ",", "@", "<", "_", "=", "?", "~", "😀", " ", "x" * 20]
        name = "".join(rng.choice(pieces) for _ in range(rng.randrange(0, 12)))
    elif r < 0.7:
        name = "".join(rng.choice(["é", "a", " ", "日", "€", "😀", "z"]) for _ in range(rng.randrange(20, 70)))
    else:
        name = nm_name()
    local = rng.choice(["a", "a.b", ".a", "a.", "a..b", "a b", "a\"b", "ü", "a@b", "", "a\x01", "a\\b", "(x)", "a.b.c", "!#$"])
    addr = local + rng.choice(["@x.com", "@", "", "@[1.2.3.4]", "@x@y"])
    nb = name.encode()
    if rng.random() < 0.1:
        nb += rng.choice([b"\xff", b"\xc3", b"\xe2\x82"])
    return f"s {hb(nb)} {hb(addr.encode())}"


def gonetmail_line():
    r = rng.random()
    if r < 0.35:
        return f"a {hb(nm_noise(nm_address()))}"
    if r < 0.6:
        n = rng.randrange(1, 4)
        parts = [nm_address() for _ in range(n)]
        seps = [rng.choice([", ", ",", " , ", ",,", ", ,", " "]) for _ in range(n)]
        text = "".join(p + sp for p, sp in zip(parts, seps))
        if rng.random() < 0.6:
            text = text[: -len(seps[-1])]
        return f"l {hb(nm_noise(text))}"
    if r < 0.85:
        return f"d {hb(nm_date())}"
    return nm_string_case()


family("gonetmail", "gonetmail", [gonetmail_line() for _ in range(lines_for("gonetmail", 16000))],
       ["env", "TZ=UTC", go_tool("gonetmail", os.path.join(LIB, "gonetmail/oracle.go"))])


# --- Java's URLEncoder and URLDecoder, against Java 17 --------------------------------------------------------

JU_CHARS = list("aZ09-_.*~'!() +%&=/?#@:;,\"<>\\^`{|}[]$\t\n") + ["é", "€", "😀", "日", "\u0000", "\u007f", "\u0085", " ", "﻿",
                                                                   "�", "٣", "Ａ", "\U0010ffff", "ß"]
JU_ESC = ["%41", "%7e", "%7E", "%2B", "%20", "%25", "%C3%A9", "%c3%a9", "%E2%82%AC", "%F0%9F%98%80", "%C3%28", "%E2%82", "%ED%A0%80",
          "%F0%9F%98", "%F4%90%80%80", "%F8", "%C0%AF", "%80", "%E0%80%80", "%EF%BF%BF", "%F0%80%80%80", "%F4%8F%BF%BF", "%E2%28%A1",
          "%F0%9F%28%80", "%F0%28", "%C2", "%E0%A0", "%F1%80", "%FF%FE", "%+F", "%-1", "%-0", "%+-", "% 1", "%٣٣", "%ＡＢ", "%Ａ1", "%ｆf",
          "%", "%4", "%😀", "%a😀", "%zz", "%g0", "%0g", "%-", "%+", "%--", "%1 ", "%٠١", "%৪৪", "%E2%82%AC%"]


def ju_text():
    return "".join(rng.choice(JU_CHARS) for _ in range(rng.randrange(0, 10)))


def ju_escaped(c):
    return "".join("%" + rng.choice(["%02X", "%02x"]) % b for b in c.encode())


def javaurlencoder_line():
    if rng.random() < 0.3:
        return f"e {hx(ju_text())}"
    parts = []
    for _ in range(rng.randrange(0, 8)):
        r = rng.random()
        if r < 0.35:
            parts.append(rng.choice(JU_ESC[:29]))
        elif r < 0.5:
            parts.append(rng.choice(JU_ESC))
        elif r < 0.7:
            parts.append(ju_escaped(rng.choice(JU_CHARS)))
        else:
            parts.append(rng.choice(JU_CHARS).replace("%", "+"))
    return f"d {hx(''.join(parts))}"


family("javaurlencoder", "javaurlencoder", [javaurlencoder_line() for _ in range(lines_for("javaurlencoder", 12000))],
       ["java", "-cp", JAVA, "B58UrlOracle"])


# --- Java's DateTimeFormatter.ofPattern(p, Locale.US), against Java 17 ----------------------------------------

DF_COMMON = "yuMLdDEecahHkKmsSnNAGQqwWYFgB"
DF_ZONE = "zVvZOXx"
DF_UNKNOWN = "bfijlortCIJPRTU"


DF_COUNTS = {"y": [1, 2, 3, 4, 5, 6], "u": [1, 2, 3, 4, 5], "M": [1, 2, 3, 4, 5], "L": [1, 2, 3, 4, 5], "Q": [1, 2, 3, 4, 5], "q": [1, 2, 3, 4, 5],
             "d": [1, 2], "h": [1, 2], "H": [1, 2], "k": [1, 2], "K": [1, 2], "m": [1, 2], "s": [1, 2], "D": [1, 2, 3], "E": [1, 2, 3, 4, 5],
             "e": [1, 2, 3, 4, 5], "c": [1, 3, 4, 5], "a": [1], "G": [1, 2, 3, 4, 5], "S": [1, 2, 3, 6, 9], "n": [1, 2, 4, 9], "N": [1, 2, 6],
             "A": [1, 2, 8], "g": [1, 2, 5], "w": [1, 2], "W": [1], "Y": [1, 2, 3, 4, 5], "F": [1], "B": [1, 4, 5]}


def df_letters():
    r = rng.random()
    if r < 0.91:
        c = rng.choice(DF_COMMON)
    elif r < 0.97:
        c = rng.choice(DF_ZONE)
    else:
        c = rng.choice(DF_UNKNOWN)
    if c in DF_COUNTS and rng.random() < 0.95:
        n = rng.choice(DF_COUNTS[c])
    else:
        n = rng.choice([1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 18, 19, 20, 21])
    run = c * n
    if rng.random() < 0.06:
        run = "p" * rng.choice([1, 2, 3, 5, 10]) + run
    return run


def df_token(depth):
    r = rng.random()
    if r < 0.55:
        return df_letters()
    if r < 0.75:
        return rng.choice([" ", "-", ":", "/", ".", ",", " ", "-", "é", "日", "(", ")", "+", "_"])
    if r < 0.85:
        return rng.choice(["'T'", "'at'", "''", "'it''s'", "'o''clock'", "' '", "'[x]'"])
    if r < 0.95 and depth < 3:
        inner = "".join(df_token(depth + 1) for _ in range(rng.randrange(0, 4)))
        return "[" + inner + ("]" if rng.random() < 0.9 else "")
    if rng.random() < 0.5:
        return df_letters()
    return rng.choice(["]", "{", "}", "#", "'unclosed", "p", "p'x'", "pp["])


DF_COMMON_PATTERNS = ["yyyy-MM-dd", "uuuu-MM-dd'T'HH:mm:ss.SSS", "EEEE, MMMM d, yyyy", "h:mm a", "dd/MM/yy", "yyyyMMddHHmmss", "QQQ yyyy",
                      "YYYY-'W'ww-e", "MMM d, uuuu h:mm:ss a", "[yyyy][-MM][-dd]", "HH:mm[:ss[.SSSSSSSSS]]", "GGGG yyyy", "B", "D", "ppd/ppM",
                      "yyyy-MM-dd'T'HH:mm:ssXXX", "EEE, dd MMM yyyy HH:mm:ss z", "W/F/g", "A n N", "kk KK hh HH"]


def df_pattern():
    if rng.random() < 0.15:
        return rng.choice(DF_COMMON_PATTERNS)
    return "".join(df_token(0) for _ in range(rng.randrange(1, 6)))


def df_year():
    r = rng.random()
    if r < 0.55:
        return rng.randrange(1, 10000)
    if r < 0.7:
        return rng.choice([0, -1, 1, 999, 1000, 9999, 10000, -999, -1000, -9999, -10000, 99, 100, 2000, 1970])
    if r < 0.85:
        return rng.randrange(-10000, 1)
    if r < 0.97:
        return rng.choice([1, -1]) * rng.randrange(10000, 1000000000)
    return rng.choice([999999999, -999999999, 999999998, -999999998])


def df_value():
    y = df_year()
    m = rng.randrange(1, 13)
    leap = y % 4 == 0 and (y % 100 != 0 or y % 400 == 0)
    dim = [31, 29 if leap else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1]
    d = rng.choice([1, dim, rng.randrange(1, dim + 1), rng.randrange(1, dim + 1)])
    if rng.random() < 0.2:
        m, d = rng.choice([(1, 1), (12, 31), (1, 2), (12, 30), (1, 7), (12, 25), (1, 3), (12, 29), (12, 28), (1, 4), (1, 5), (12, 27)])
    h = rng.choice([0, 12, 23, rng.randrange(0, 24), rng.randrange(0, 24), rng.randrange(0, 24)])
    mi = rng.choice([0, 0, 59, rng.randrange(0, 60), rng.randrange(0, 60)])
    sec = rng.choice([0, 0, 59, rng.randrange(0, 60)])
    nano = rng.choice([0, 0, 1, 999999999, 123000000, 123456000, 123456789, 500000000, rng.randrange(0, 1000000000)])
    return y, m, d, h, mi, sec, nano


def javadatetimeformatter_line():
    kind = rng.choice(["D", "T", "DT", "DT", "DT"])
    y, m, d, h, mi, sec, nano = df_value()
    return f"{hx(df_pattern())} {kind} {y} {m} {d} {h} {mi} {sec} {nano}"


family("javadatetimeformatter", "javadatetimeformatter", [javadatetimeformatter_line() for _ in range(lines_for("javadatetimeformatter", 14000))],
       ["java", "-cp", JAVA, "B58DtfOracle"])


# --- Go's html escaping, against Go 1.25 ---------------------------------------------------------------------

import html.entities as _html_entities

GH_NAMES = sorted(_html_entities.html5)
GH_LEGACY = [n for n in GH_NAMES if not n.endswith(";")]


def gh_ref():
    r = rng.random()
    if r < 0.3:
        return "&" + rng.choice(GH_NAMES)
    if r < 0.45:
        return "&" + rng.choice(GH_LEGACY) + rng.choice(["x", "1", "X;", "y;", "", ";", "=", "ab;", "z"])
    if r < 0.55:
        n = rng.choice(GH_NAMES).rstrip(";")
        return "&" + n[:rng.randrange(1, len(n) + 1)] + rng.choice(["", ";", "x;"])
    if r < 0.85:
        hexed = rng.random() < 0.4
        v = rng.choice([0, 1, 9, 38, 60, 65, 127, 128, 133, 150, 159, 160, 0xd800, 0xdfff, 0xfffd, 0x10ffff, 0x110000, 0x1f600, 2 ** 31 - 1,
                        2 ** 31, 2 ** 32 + 65, 2 ** 32 - 1, 10 ** 20 + 7, rng.randrange(0, 200000), rng.randrange(0, 1 << 40)])
        digits = ("%x" % v if hexed else str(v))
        if rng.random() < 0.2:
            digits = digits.upper()
        if rng.random() < 0.15:
            digits = "0" * rng.randrange(1, 5) + digits
        if rng.random() < 0.1:
            digits = digits[:1]
        return "&#" + (rng.choice(["x", "X"]) if hexed else "") + digits + rng.choice([";", ";", ";", "", "x", " ", "g;", "&"])
    return rng.choice(["&", "&&", "&#", "&#x", "&#;", "&#x;", "&;", "& ", "&#a;", "&#xg;", "&1;", "&Amp;", "&AMP", "&amp", "&ampamp;", "&notin",
                       "&notit;", "&#x80", "&#X9F;", "&#38;#38;", "&nGt;", "&nLt;", "&nGtv;", "&nLtv;"])


def gh_text():
    parts = []
    for _ in range(rng.randrange(0 if rng.random() < 0.03 else 1, 7)):
        r = rng.random()
        if r < 0.5:
            parts.append(gh_ref())
        else:
            parts.append(rng.choice(["a", " ", "<", ">", "\"", "'", "x;y", "é", "日本", "😀", "0", "#", ";", "\n", "abc", "amp"]))
    b = "".join(parts).encode()
    if rng.random() < 0.05:
        i = rng.randrange(0, len(b) + 1)
        b = b[:i] + rng.choice([b"\xff", b"\xc3", b"\x80"]) + b[i:]
    return b


GH_ESC = ["a", "&", "<", ">", '"', "'", "é", " ", "&amp;", "😀", ";"]


def gohtml_line():
    if rng.random() < 0.2:
        return "e " + hx("".join(rng.choice(GH_ESC) for _ in range(rng.randrange(0 if rng.random() < 0.03 else 1, 10))))
    return f"u {hb(gh_text())}"


family("gohtml", "gohtml", [gohtml_line() for _ in range(lines_for("gohtml", 12000))],
       [go_tool("gohtml", os.path.join(LIB, "gohtml/oracle.go"))])


# --- Go's net/http cookies, against Go 1.25 (TZ=UTC) ---------------------------------------------------------

HC_NAMES = ["a", "sid", "SESSION_ID", "x-y", "__Host-id", "a.b", "!#$%&'*+-.^_`|~"]
HC_NAMES_ODD = ["", "a b", "(x)", "é", "a=b", "a\"b", "a,b", "a\tb", "a@b", "[x]"]
HC_VALUES = ["v", "", "abc123", "\"quoted\"", "\"\"", "a b", "a,b", "x=y", "!#$%&'()*+-./:<=>?@[]^_`{|}~", "\"a b\"", " lead", "trail "]
HC_VALUES_ODD = ["a\"b", "a\\b", "é", "\x01", "\x7f", "\"unclosed", "\"", "a\"", "\"a\"b\""]


def hc_pick(ok, odd, p=0.85):
    return rng.choice(ok) if rng.random() < p else rng.choice(odd)


def hc_pair():
    r = rng.random()
    if r < 0.04:
        return rng.choice(["", " ", "noequals", "=v", "=", "a"])
    return hc_pick(HC_NAMES, HC_NAMES_ODD, 0.95) + rng.choice(["="] * 12 + [" =", "= "]) + hc_pick(HC_VALUES, HC_VALUES_ODD, 0.93)


def hc_cookie_line():
    if rng.random() < 0.005:
        return ";".join("a=b" for _ in range(rng.choice([3000, 3001])))
    seps = [";", "; ", " ; ", ";  ", ";\t"]
    line = "".join(hc_pair() + rng.choice(seps) for _ in range(rng.randrange(1, 5)))
    if rng.random() < 0.85:
        line = line.rstrip(" \t").rstrip(";")
    return rng.choice(["", "", " ", "\t", "\r\n"]) + line + rng.choice(["", "", " ", "\n"])


HC_DATES = ["Wed, 21 Oct 2015 07:28:00 GMT", "Thu, 01 Jan 1970 00:00:00 GMT", "Mon, 02-Jan-2006 15:04:05 MST", "Sat, 31 Dec 1600 23:59:59 GMT",
            "Mon, 01 Jan 1601 00:00:00 GMT", "Fri, 31 Dec 9999 23:59:59 GMT", "Sun, 06 Nov 1994 08:49:37 PST", "Mon, 01 Jan 0001 00:00:00 GMT",
            "Tue, 15 Nov 1994 12:45:26 GMT+3", "Tue, 15 Nov 1994 12:45:26 UTC", "Wed, 21 Oct 2015 07:28:00", "21 Oct 2015 07:28:00 GMT",
            "Wednesday, 21-Oct-15 07:28:00 GMT", "Wed, 32 Oct 2015 07:28:00 GMT", "Wed, 21 Oct 2015 7:28:00 GMT", "Wed, 21 Oct 2015 07:28:00 gmt",
            "Wed, 21-Oct-2015 07:28:00 GMT", "Wed, 21 Oct 2015 07:28:00.123 GMT", "Wed, 21 Oct 2015 24:00:00 GMT", "0"]


def hc_attr():
    r = rng.random()
    if r < 0.12:
        return rng.choice(["Path", "path", "PATH"]) + "=" + rng.choice(["/", "/a/b", "", "/a b", "/é", "/a\\b", "/\"q\""])
    if r < 0.24:
        return rng.choice(["Domain", "domain"]) + "=" + rng.choice(["example.com", ".example.com", "1.2.3.4", "::1", "[::1]", "a..b", "-a.com", "a-.com",
                                                                     "localhost", "x" * 64 + ".com", "1.2.3", "é.com", "", "a_b.com", "123"])
    if r < 0.38:
        return rng.choice(["Expires", "expires", "EXPIRES"]) + "=" + rng.choice(HC_DATES)
    if r < 0.52:
        return rng.choice(["Max-Age", "max-age", "MAX-AGE"]) + rng.choice(["=", "=", " = "]) + rng.choice(
            ["0", "-1", "10", "010", "+5", "abc", "", "9223372036854775807", "9223372036854775808", "-9223372036854775808", "00", "+0", "-0", "1e3"])
    if r < 0.62:
        return rng.choice(["Secure", "secure", "HttpOnly", "httponly", "Partitioned", "partitioned", "Secure=yes", "HttpOnly=", "Secure=\"x\""])
    if r < 0.74:
        return rng.choice(["SameSite", "samesite", "SAMESITE"]) + "=" + rng.choice(["Lax", "lax", "Strict", "STRICT", "None", "none", "foo", "", "é", "\"Lax\""])
    if r < 0.86:
        return rng.choice(["Foo=bar", "Comment=x", "Version=1", "x", "é=1", "Path=a;b", "a\x01=b", "Priority=High", "=x", "Max-Age"])
    return rng.choice(["", " ", "Path=/\x7f", "Domain=a\"b", "Expires=a\\b", "SameSite=a b"])


def hc_set_cookie_line():
    first = hc_pair() if rng.random() < 0.95 else rng.choice(["", " ", ";", "a"])
    line = first + "".join(rng.choice([";", "; ", " ;", ";  "]) + hc_attr() for _ in range(rng.randrange(0, 5)))
    b = (rng.choice(["", "", " ", "\t"]) + line + rng.choice(["", "", " ", ";"])).encode()
    if rng.random() < 0.03:
        i = rng.randrange(0, len(b) + 1)
        b = b[:i] + rng.choice([b"\xff", b"\xc3"]) + b[i:]
    return b


def hc_write_line():
    name = hc_pick(HC_NAMES, HC_NAMES_ODD, 0.9)
    value = hc_pick(HC_VALUES, HC_VALUES_ODD + ["a;b", "é;\"", "\x00\x01"], 0.8)
    path = hc_pick(["", "", "/", "/a b", "/x/y"], ["/a;b", "/é", "/\x7f"], 0.85)
    domain = hc_pick(["", "", "example.com", ".example.com", "1.2.3.4", "localhost"], ["::1", "a..b", "é.com", "-x.com", "x" * 64 + ".org", "1.2.3.4%x"], 0.85)
    exp = rng.choice(["-", "-", "0.0", "1445412480.0", "-62135596800.0", "-11644473601.999999999", "-11644473600.0", "253402300799.5",
                      str(rng.randrange(-70000000000, 300000000000)) + "." + str(rng.randrange(0, 1000000000))])
    secure = rng.choice("01")
    partitioned = secure if rng.random() < 0.7 else rng.choice("01")
    return " ".join(["w", hx(name), hx(value), rng.choice("01"), hx(path), hx(domain), exp, str(rng.choice([0, 0, 1, -1, 3600, -5])),
                     secure, rng.choice("01"), str(rng.randrange(0, 5)), partitioned])


def gohttpcookie_line():
    r = rng.random()
    if r < 0.3:
        return f"c {hx(hc_cookie_line())}"
    if r < 0.8:
        return f"s {hb(hc_set_cookie_line())}"
    return hc_write_line()


family("gohttpcookie", "gohttpcookie", [gohttpcookie_line() for _ in range(lines_for("gohttpcookie", 14000))],
       ["env", "TZ=UTC", go_tool("gohttpcookie", os.path.join(LIB, "gohttpcookie/oracle.go"))])


# --- Java's SimpleDateFormat(p, Locale.US) in GMT, against Java 17 -------------------------------------------

SD_LETTERS = "GyMdkHmsSEDFwWahKzZYuXL"
SD_ILLEGAL = "bcefgijlnopqrtvxABCIJNOPQRTUV"
SD_MIN = -12181017600000          # 1584-01-01T00:00:00Z
SD_9999 = 253402300799999         # 9999-12-31T23:59:59.999Z
SD_QUOTED = ["'T'", "'at'", "''", "'it''s'", "'o''clock'", "' '", "'yyyy'", "'é'", "''" + "''"]


def sd_token():
    r = rng.random()
    if r < 0.6:
        c = rng.choice(SD_LETTERS)
        n = rng.choice([1, 1, 2, 2, 3, 4, 5]) if rng.random() < 0.95 else rng.choice([6, 9, 20, 254, 255, 256, 310])
        if c == "X" and rng.random() < 0.8:
            n = rng.choice([1, 2, 3])
        return c * n
    if r < 0.78:
        return rng.choice([" ", "-", ":", "/", ".", ",", "é", "日本", "(", "+", " "])
    if r < 0.93:
        return rng.choice(SD_QUOTED)
    return rng.choice(["'unclosed", rng.choice(SD_ILLEGAL), "XXXX", "XXXX-", "XXXXb", "'a''"])


SD_COMMON = ["yyyy-MM-dd'T'HH:mm:ss.SSSZ", "EEE, d MMM yyyy HH:mm:ss Z", "yyyyMMddHHmmss", "dd/MM/yy", "h:mm a", "EEEE, MMMM d, yyyy",
             "YYYY-'W'ww-u", "yyyy-MM-dd HH:mm:ss zzzz", "K:mm a, z", "D F W w", "kk:mm", "yyyy.MM.dd G 'at' HH:mm:ss z", "LLLL", "MMM"]


def sd_pattern():
    if rng.random() < 0.12:
        return rng.choice(SD_COMMON)
    return "".join(sd_token() for _ in range(rng.randrange(1, 6)))


def sd_day(y, m, d):
    import datetime
    try:
        return (datetime.date(y, m, d) - datetime.date(1970, 1, 1)).days
    except ValueError:
        return (datetime.date(y, 3, 1) - datetime.date(1970, 1, 1)).days


def sd_millis():
    r = rng.random()
    if r < 0.5:
        return rng.randrange(SD_MIN, SD_9999)
    if r < 0.75:
        y = rng.randrange(1584, 10000)
        m, d = rng.choice([(1, 1), (1, 2), (1, 3), (1, 5), (1, 7), (12, 25), (12, 28), (12, 29), (12, 30), (12, 31), (2, 29), (3, 1)])
        return sd_day(y, m, d) * 86400000 + rng.choice([0, 1, 43200000, 86399999, rng.randrange(0, 86400000)])
    if r < 0.85:
        return rng.choice([SD_MIN, SD_MIN + 1, 0, -1, 1, 86399999, 9223372036854775807, 951782400000, 946684799999])
    return rng.randrange(SD_9999, 9223372036854775807)


def javasimpledateformat_line():
    return f"{hx(sd_pattern())} {sd_millis()}"


family("javasimpledateformat", "javasimpledateformat", [javasimpledateformat_line() for _ in range(lines_for("javasimpledateformat", 12000))],
       ["java", "-cp", JAVA, "B58SdfOracle"])


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
