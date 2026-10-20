"""B62's packages against the references they reproduce.

- Ruby 2.6's CGI.escape/unescape/escapeHTML/unescapeHTML/escapeElement/
  unescapeElement/pretty/rfc1123_date, CGI.parse, CGI::Cookie, ERB::Util.
- Node 25's path.posix/win32.matchesGlob (its minimatch 10), nocase on and off.
- Ruby 2.6's CSV.parse_line and CSV.generate_line (csv 3.0.9).
- Ruby 2.6's Rational: parsing, arithmetic, rounding, rationalize.
- Ruby 2.6's Date under its calendar reform.
- Ruby 2.6's Array#pack, String#unpack and #unpack1 (no float or pointer directives).
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.environ.get("B62_ROOT") or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PKG = os.environ.get("B62_PKG") or os.path.join(ROOT, "packages")
LIB = os.environ.get("B62_LIB") or os.path.join(ROOT, "scripts/lib/b62")
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b62")
harness = os.path.join(work, "b62test")
built = subprocess.run([GRASM, "build", os.path.join(PKG, "b62test/src/main.gr"), "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
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


# --- Ruby's CGI and ERB::Util, against Ruby 2.6 -------------------------------------------------------------------

CG_BYTES = [b"a", b"Z", b"0", b" ", b"-", b".", b"_", b"~", b"*", b"+", b"%", b"&", b"=", b";", b"/", b"?", b"#", b"<", b">", b"\"", b"'", b"\n", b"\x00",
            "é".encode(), "😀".encode(), b"\xff", b"\xc3"]
CG_ESC = [b"%41", b"%e9", b"%C3%A9", b"%", b"%4", b"%zz", b"%%41", b"+", b"%2B", b"%ff", b"%+", b"%4+", b"+%"]
CG_ENT = [b"&amp;", b"&apos;", b"&quot;", b"&gt;", b"&lt;", b"&#65;", b"&#x41;", b"&#X41;", b"&#1114111;", b"&#1114110;", b"&#55296;", b"&#0;", b"&#00065;",
          b"&#x;", b"&#;", b"&#", b"&", b"&a", b"&am", b"&ampx", b"&AMP;", b"&#12", b"&#99999999999999999999999;", b"&#x10FFFF;", b"&#xD800;", b"&#xe9;",
          b"&#233", b"&q", b"&g", b"&lt", b"&#x1F600;", b"&&amp;", b"&#&amp;", b"&#x&lt;"]
CG_TAGS = [b"<a>", b"</a>", b"<A href=1>", b"</A>", b"<b>", b"</b>", b"<br>", b"<br/>", b"<abbr>", b"</abbr>", b"<p>", b"</p>", b"<a\n>", b"<a", b">", b"x", b" ",
           b"\n", b"&lt;a&gt;", b"&lt;/a&gt;", b"&LT;b&GT;", b"&lt;br/&gt;", b"&lt;a", b"text", b"<_>", b"<a1>"]


def cg_bytes(pieces, n=6):
    return b"".join(rng.choice(pieces) for _ in range(rng.randrange(0, n)))


def cg_names():
    return [rng.choice(["a", "b", "br", "A", "p", "abbr", "x"]) for _ in range(rng.choice([0, 1, 1, 2, 3]))]


def rubycgi_line():
    h = lambda b: b.hex() or "-"
    r = rng.random()
    if r < 0.15:
        return f"{rng.choice(['es', 'ue', 'eh', 'hh'])} {h(cg_bytes(CG_BYTES))}"
    if r < 0.25:
        return f"un {h(cg_bytes(CG_BYTES + CG_ESC * 2))}"
    if r < 0.45:
        return f"uh {h(cg_bytes(CG_ENT * 2 + [b'x', b'a', b'#', b';', b'&', b' ', b'1']))}"
    if r < 0.6:
        return (f"{rng.choice(['ee', 'ce'])} {h(cg_bytes(CG_TAGS, 8))} " + " ".join(n.encode().hex() for n in cg_names())).rstrip()
    if r < 0.72 and rng.random() < 0.5:
        def tree(d):
            if d > 2 or rng.random() < 0.3:
                return rng.choice([b"x", b"y z", b"", b"\n", b"<br>"])
            name = rng.choice([b"a", b"b", b"ul", b"li", b"p", b"P"])
            return b"<" + name + b">" + b"".join(tree(d + 1) for _ in range(rng.randrange(0, 3))) + b"</" + (name if rng.random() < 0.9 else name.upper()) + b">"
        return f"pr {h(tree(0) + (tree(0) if rng.random() < 0.3 else b''))} {h(rng.choice([b'  ', b'  ', b'\t', b'--', b' ']))}"
    if r < 0.72:
        html = cg_bytes([b"<a>", b"</a>", b"<b>", b"</b>", b"<br>", b"<p>", b"</p>", b"<P>", b"x", b"y z", b"\n", b"<ul>", b"</ul>", b"<li>", b"</li>", b"<",
                         b">", b"  "], 10)
        return f"pr {h(html)} {h(rng.choice([b'  ', b'  ', b'\t', b'--', b' ', b'ab']))}"
    if r < 0.84:
        return f"pa {h(cg_bytes([b'a', b'b', b'=', b'&', b';', b'%41', b'+', b'x=1', b'a=2', b'%zz', b'=', b'&&', 'é'.encode()], 8))}"
    if r < 0.92:
        return f"cp {h(cg_bytes([b'a', b'b', b'=', b'&', b';', b'; ', b';  ', b';\t', b'%41', b'+', b'x=1', b'a=2', b'%zz', b'=='], 8))}"
    if r < 0.97:
        vals = [h(cg_bytes(CG_BYTES, 3)) for _ in range(rng.randrange(0, 3))]
        return (f"ct {h(rng.choice([b'a', b'sid', b'x y', b'']))} {'.'.join(vals) if vals else '~'} {rng.choice(['n', h(b'ex.com')])} {h(rng.choice([b'', b'/', b'/a b']))} "
                f"{rng.choice(['n', str(rng.randrange(-10 ** 11, 10 ** 11))])} {rng.choice('01')} {rng.choice('01')}")
    return f"rd {rng.choice([0, -1, 86399, -62198755200, 951782400, 253402300799, rng.randrange(-10 ** 12, 10 ** 12)])}"


family("rubycgi", "rubycgi", [rubycgi_line() for _ in range(int(os.environ.get("RUBYCGI_LINES", "12000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubycgi/oracle.rb")])

# --- Node's path.matchesGlob (minimatch 10), against Node 25 ----------------------------------------------------

MG_LIT = ["a", "b", "A", "B", "x", "é", "É", "ſ", "\u212a", "k", "K", "😀", " ", "-", ",", "#", ".", "..", "_", "1", "٣", "\t", "·", "\u3000", "$", "^", "|",
          "(", ")", "+", "@", "!", "{", "}", "]", "ǅ", "ß", "İ", "Σ", "ς", "s", "S"]
MG_MAGIC = ["*", "*", "**", "?", "??", "*.", ".*", "*.*", "*a", "?.b", "***"]
MG_CLASS = ["[ab]", "[!a]", "[^a]", "[a-c]", "[c-a]", "[]a]", "[!]a]", "[a-]", "[-a]", "[]", "[!]", "[a", "[[:alpha:]]", "[[:digit:]x]", "[[:upper:]]",
            "[[:lower:]]", "[[:space:]]", "[[:punct:]]", "[![:alnum:]]", "[[:ascii:]]", "[[:xdigit:]]", "[[:graph:]]", "[[:print:]]", "[[:word:]]",
            "[[:blank:]]", "[[:cntrl:]]", "[x-[:alpha:]]", "[é-ſ]", "[😀]", "[a-😀]", "[.]", "[A-Z]", "[k]", "[K]", "[\u212a]", "[[:alpha:]-]", "[[:foo:]]",
            "[a\n]", "[s]", "[^[:upper:]]", "[[:alpha:][:digit:]]", "[!.]"]
MG_EXT = ["@(a|b)", "!(a)", "!(a|)", "+(a|*)", "*(a|b)", "?(x)", "@()", "*()", "!()", "+(|)", "@(@())", "@(*())", "!(x@(y))", "@(a", "!(*.x)",
          "+(@(a|b)|c)", "!(!(a))", "@(A|é)", "*([[:alpha:]])", "!(.*)", "+(.a|b)", "@(x)y", "?(a|.b)", "*(.)", "!(b)*()", "@(a|[b-c])", "!(a)b",
          "+(a)", "*(a)", "?(*)", "@(.)", "!([a)])", "@(k|S)", "!(*)"]
MG_BRACE = ["{a,b}", "{a,*}", "{,x}", "{1..3}", "{a..c}", "{a}", "{x,{y,z}}", "${a,b}", "{a,b", "{}", "{..}", "{Z..a}", "{01..3}", "{-1..1}", "{a..e..2}",
            "{.,..}", "{a,b}{c,}", "{**,x}", "{*,a}", "{a,a}", "{,}", "{a\n,b}"]
MG_ALPH = ["a", "b", "A", "x", "k", "K", "s", "S", ".", "-", "é", "É", "ſ", "\u212a", "😀", " ", ",", "_", "1", "٣", "\t", "·", "\u3000", "ǅ", "ß", "İ",
           "Σ", "ς", "y", "c"]


def mg_rand(lo, hi):
    return "".join(rng.choice(MG_ALPH) for _ in range(rng.randint(lo, hi)))


def mg_segment():
    pieces = []
    for _ in range(rng.choice([1, 1, 2, 2, 3])):
        r = rng.random()
        pool = MG_LIT if r < 0.4 else MG_MAGIC if r < 0.6 else MG_CLASS if r < 0.73 else MG_EXT if r < 0.88 else MG_BRACE
        pieces.append(rng.choice(pool))
    return "".join(pieces)


def mg_pattern(windows):
    if rng.random() < 0.02:
        return ""
    segs = [mg_segment() for _ in range(rng.choice([1, 1, 2, 2, 3, 4]))]
    if rng.random() < 0.15:
        segs.insert(rng.randrange(len(segs) + 1), rng.choice([".", "..", "", "**"]))
    seps = ["/", "/", "/", "\\", "//"] if windows else ["/", "/", "/", "\\", "//"]
    out = segs[0]
    for s in segs[1:]:
        out += rng.choice(seps) + s
    prefix = rng.choice(["", "", "", "", "/", "./", "../", "**/"] + (["c:/", "C:\\", "//?/c:/", "//srv/share/", "\\\\?\\C:\\", "//"] if windows else ["//"]))
    return prefix + out


def mg_close(p, i):
    depth = 0
    for j in range(i, len(p)):
        if p[j] == "(":
            depth += 1
        elif p[j] == ")":
            depth -= 1
            if depth == 0:
                return j
    return -1


def mg_alts(body):
    out, depth, cur = [], 0, ""
    for ch in body:
        if ch == "|" and depth == 0:
            out.append(cur)
            cur = ""
            continue
        depth += ch == "("
        depth -= ch == ")"
        cur += ch
    return out + [cur]


def mg_flip(ch):
    if rng.random() < 0.15:
        return {"é": "É", "É": "é", "ſ": "S", "\u212a": "k", "k": "\u212a", "ς": "Σ", "Σ": "σ", "ß": "ẞ"}.get(ch, ch.swapcase() if ch.isascii() else ch)
    return ch


def mg_inst(p, depth=0):
    out, i = [], 0
    while i < len(p):
        c = p[i]
        if c in "@!+*?" and p.startswith("(", i + 1) and depth < 3:
            j = mg_close(p, i + 1)
            if j > 0:
                alts = mg_alts(p[i + 2:j])
                if c == "!":
                    out.append(mg_rand(0, 2))
                else:
                    n = {"@": 1, "?": rng.choice([0, 1]), "+": rng.choice([1, 2]), "*": rng.choice([0, 1, 2])}[c]
                    out.append("".join(mg_inst(rng.choice(alts), depth + 1) for _ in range(n)))
                i = j + 1
                continue
        if c == "*":
            if p.startswith("**", i):
                out.append(rng.choice(["", "a", "a/b", "x/.y", "b/c/d", "A"]))
                i += 2
                continue
            out.append(mg_rand(0, 3))
        elif c == "?":
            out.append(mg_rand(1, 1))
        elif c == "[" and p.find("]", i + 2) > 0:
            j = p.find("]", i + 2)
            inner = [ch for ch in p[i + 1:j] if ch not in "!^-[]:"]
            out.append(rng.choice(inner) if inner and rng.random() < 0.7 else mg_rand(1, 1))
            i = j + 1
            continue
        elif c == "{" and p.find("}", i) > 0:
            j = p.find("}", i)
            out.append(rng.choice(p[i + 1:j].split(",")))
            i = j + 1
            continue
        else:
            out.append(mg_flip(c))
        i += 1
    return "".join(out)


MG_SEGS = ["**", "**", "..", ".", "", "*", "*", "a", "b", "?", "{a,b}", "{a,*}", "*.x", ".*", "[ab]", "A", "c:", "@(a|b)", "!(a)", "{.,a}", "{**,a}"]
MG_NAMES = ["a", "a", "b", "..", ".", "", ".x", "A", "a.x", "c:", "C:", "?", "**", "B", "x"]


def mg_struct_line(op):
    """Paths and patterns of whole segments: the simplifications, `**`, dots, drives and UNC roots."""
    sep = "\\" if op[0] == "w" and rng.random() < 0.3 else "/"
    pat = sep.join(rng.choice(MG_SEGS) for _ in range(rng.randint(1, 5)))
    if rng.random() < 0.15:
        pat = sep.join([rng.choice(["a", "*", "**", "."]), "**", "..", rng.choice(["a", "*", "b", "..", "?"]), rng.choice(["b", "*", "a", "."])])
    if rng.random() < 0.5:
        path = sep.join(rng.choice(MG_NAMES) for _ in range(rng.randint(1, 5)))
    else:
        path = "/".join(rng.choice(["a", "b", "x", ".y", "A"]) if s in ("*", "**", "?", "[ab]", "{a,*}", "!(a)") else
                        rng.choice(s[1:-1].split(",")) if s.startswith("{") else "a" if s.startswith("@") else s for s in pat.split(sep))
    pre = rng.choice(["", "", "", "/", "//", "c:/", "C:/", "//?/c:/", "//?/C:/", "//s/h/", "./"])
    if rng.random() < 0.5:
        return pre + path, pre + pat
    return rng.choice(["", "", "/", "c:/", "//?/c:/", "//s/h/"]) + path, pre + pat


def nodematchesglob_line():
    op = rng.choice(["px", "wx", "pl", "wl"])
    windows = op[0] == "w"
    h = lambda t: t.encode().hex() or "-"
    if rng.random() < 0.3:
        path, pat = mg_struct_line(op)
        return f"{op} {h(path)} {h(pat)}"
    pat = mg_pattern(windows)
    if rng.random() < 0.6:
        path = mg_inst(pat)
    else:
        path = "/".join(mg_rand(0, 3) for _ in range(rng.choice([1, 1, 2, 3])))
    if rng.random() < 0.1:
        path = rng.choice(["./", "../", "a/../", "/", "//", "c:/", "C:\\", "//?/C:/", "//srv/share/"]) + path
    if windows and rng.random() < 0.3:
        path = path.replace("/", "\\")
    return f"{op} {h(path)} {h(pat)}"


family("nodematchesglob", "nodematchesglob", [nodematchesglob_line() for _ in range(int(os.environ.get("NODEMATCHESGLOB_LINES", "12000")))],
       ["node", "--expose-internals", os.path.join(LIB, "nodematchesglob/oracle.js")])

# --- Ruby's CSV.parse_line and CSV.generate_line (csv 3.0.9), against Ruby 2.6 ------------------------------------

CV_PIECES = ["a", "b", "x y", "é", "1", " ", "  ", "\t", "\\", "\v", "\0", "\f", "z", "😀", "ab"]


def cv_field(q, col, rs):
    r = rng.random()
    body = "".join(rng.choice(CV_PIECES + [q, col, rs, "\n", "\r", "\r\n", q + q]) for _ in range(rng.randint(0, 3)))
    if r < 0.45:
        return "".join(rng.choice(CV_PIECES) for _ in range(rng.randint(0, 3)))
    if r < 0.8:
        return q + body.replace(q, q + q) + q
    if r < 0.9:
        return q + body + rng.choice(["", q, q + "x", q + q])
    return body


def cv_options():
    while True:
        col = rng.choice([",", ",", ",", ";", "\t", " ", "||", "ab", "¦", ",,"])
        q = rng.choice(['"'] * 12 + ["'", "'", "|", "|", None, None, None, "ab"] + [""])
        rs = rng.choice([None, None, None, "\n", "\r\n", "\r", "|", "XY"])
        strip = rng.choice([False] * 14 + [True, True, True, " ", " ", "x", "x", "", "ab"])
        chars = [set(col), set(q or ""), set(rs or ""), set(strip) if isinstance(strip, str) else set()]
        if any(chars[i] & chars[j] for i in range(4) for j in range(i + 1, 4)):
            continue
        if q is None and isinstance(strip, str) and strip and col.startswith(strip):
            continue
        return col, q, rs, strip


def rubycsvline_line():
    h = lambda t: t.encode().hex() or "-"
    col, q, rs, strip = cv_options()
    if rng.random() < 0.25:
        qq = q if q and len(q) == 1 else '"'
        row = "~" if rng.random() < 0.05 else ".".join("n" if rng.random() < 0.15 else h(cv_field(qq, col, "\n").replace(qq * 2, qq) if rng.random() < 0.5 else
                                                                                         "".join(rng.choice(CV_PIECES + [col, qq, "\n", "\r"]) for _ in range(rng.randint(0, 3))))
                                                       for _ in range(rng.randint(1, 4)))
        return (f"g {row} {h(col)} {h(rng.choice(['\n', '\r\n', '|', '']))} {h(qq)} {rng.choice('001')} {rng.choice('011')} "
                f"{rng.choice(['n', 'n', h('N'), '-'])} {rng.choice(['-', '-', h('E')])}")
    qc = q if q and len(q) == 1 else '"'
    seps = [rs] if rs else ["\n", "\r\n", "\r"]
    lines = []
    for _ in range(rng.choice([1, 1, 2, 3])):
        fields = [cv_field(qc, col, seps[0]) for _ in range(rng.randint(0, 4))]
        joiner = col if rng.random() < 0.9 else rng.choice([",", ";", col[0], col + col])
        lines.append(joiner.join(fields))
    if rng.random() < 0.15:
        lines.insert(0, "")
    text = "".join(l + rng.choice(seps) for l in lines[:-1]) + lines[-1] + (rng.choice(seps) if rng.random() < 0.5 else "")
    raw = text.encode()
    if rng.random() < 0.04:
        at = rng.randrange(len(raw) + 1)
        raw = raw[:at] + rng.choice([b"\xff", b"\xc3", b"\xed\xa0\x80", b"\xe9"]) + raw[at:]
    strip_s = "f" if strip is False else "t" if strip is True else "x" + h(strip)
    return (f"p {raw.hex() or '-'} {h(col)} {'a' if rs is None else h(rs)} {'n' if q is None else h(q)} {rng.choice([0, 0, 0, 1, 2, 3, 4])} {strip_s} "
            f"{rng.choice(['n'] * 9 + ['0', '2', '4'])} {rng.choice('001')} {rng.choice(['n', 'n', h('N')])} {rng.choice(['-', '-', h('E')])}")


family("rubycsvline", "rubycsvline", [rubycsvline_line() for _ in range(int(os.environ.get("RUBYCSVLINE_LINES", "12000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubycsvline/oracle.rb")])

# --- Ruby's Rational, against Ruby 2.6 --------------------------------------------------------------------------

RR_INT = ["0", "1", "12", "007", "0_1", "1_000", "1__0", "_1", "1_", "00", "0__1", "123456789012345678901234567890", "3", "25", "0_", "7", "10", "0_0_",
          "00_1_2", "2_5", "98765432109876543210", "1x"]
RR_FRAC = ["", "5", "25", "00", "0_5", "1_2", "333", "x", "e5", "0", "125", "1__2", "5_", "0123456789"]
RR_EXP = ["0", "1", "2", "5", "12", "30", "0_1", "1_0", "", "99999999999999999999", "x", "0_", "3"]


def rr_number():
    out = ""
    if rng.random() < 0.85:
        out += rng.choice(RR_INT)
    if rng.random() < 0.4:
        out += "." + rng.choice(RR_FRAC)
    if rng.random() < 0.3:
        out += rng.choice("eE") + rng.choice(["", "", "-", "+"]) + rng.choice(RR_EXP)
    return out


def rr_clean():
    num = lambda: (rng.choice(["0", "1", "12", "1_000", "25", "3", "007", "123456789012345678901234567890"]) + (("." + rng.choice(["5", "25", "0_5", "125", "00"]))
                   if rng.random() < 0.4 else "") + ((rng.choice("eE") + rng.choice(["", "-", "+"]) + rng.choice(["0", "1", "2", "5", "12", "1_0"])) if rng.random() < 0.3 else ""))
    t = rng.choice(["", "", " ", "-", "+"]) + num()
    if rng.random() < 0.5:
        t += "/" + num()
    return t + rng.choice(["", "", "", " "])


def rr_text():
    if rng.random() < 0.5:
        return rr_clean()
    t = rng.choice(["", "", "", " ", "\t", "\n "]) + rng.choice(["", "", "", "-", "+", "--"]) + rr_number()
    if rng.random() < 0.4:
        t += rng.choice(["/", "/", "/", " /"]) + (rr_number() if rng.random() < 0.85 else rng.choice(["-2", "", ".", "0", "0.0"]))
    t += rng.choice(["", "", "", "", " ", "x", "/3", " 1", "\t", "١", "２", "é", "\0", "#{", "\""])
    return t


def rr_int(big=0.1):
    if rng.random() < big:
        return str(rng.choice([-1, 1]) * rng.randrange(10 ** 18, 10 ** 30))
    return str(rng.randint(-60, 60))


def rr_rat(zero_ok=True):
    d = rr_int(0.05)
    while d == "0":
        d = rr_int(0.05)
    if rng.random() < 0.4:
        d = str(rng.choice([2, 4, 8, 20, 200, 3, 7, -2, 16]))
    n = rr_int()
    if not zero_ok and int(n) == 0:
        n = "1"
    return f"{n}/{d}"


def rubyrational_line():
    r = rng.random()
    h = lambda t: t.encode().hex() or "-"
    if r < 0.35:
        return f"{rng.choice(['ps', 'ps', 'tr'])} {h(rr_text())}"
    if r < 0.4:
        return f"mk {rr_int()} {rng.choice([rr_int(), '0', rr_int()])}"
    if r < 0.6:
        op = rng.choice(["add", "sub", "mul", "div", "cmp", "pow"])
        if op == "pow":
            base = rng.choice([rr_rat(), "0/1", "1/1", "-1/1", rr_rat()])
            return f"ar pow {base} {rng.randint(-6, 6)}"
        return f"ar {op} {rr_rat()} {rr_rat() if op != 'div' or rng.random() < 0.9 else '0/1'}"
    if r < 0.85:
        kind = rng.choice(["floor", "ceil", "truncate", "round", "round", "round"])
        digits = rng.choice(["-", "-", "0", "1", "2", "3", "-1", "-2", "4"])
        half = rng.choice(["-", "up", "even", "down", "UP", "Even", "foo"]) if kind == "round" else "-"
        return f"rd {kind} {rr_rat()} {digits} {half}"
    if r < 0.95:
        return f"rz {rr_rat()} {rng.choice(['-', rr_rat(), '1/100', '1/10', '0/1', '-1/3'])}"
    return f"ts {rr_rat()}"


family("rubyrational", "rubyrational", [rubyrational_line() for _ in range(int(os.environ.get("RUBYRATIONAL_LINES", "12000")))],
       ["ruby", "-E", "UTF-8:UTF-8", os.path.join(LIB, "rubyrational/oracle.rb")])

# --- Ruby's Date, against Ruby 2.6 ------------------------------------------------------------------------------

RD_STARTS = ["I"] * 10 + ["E", "E", "E", "J", "J", "G", "G", "2298874", "2299170", "2300000", "2361227", "2426355", "2340000", "1000", "3000000"]


def rd_year():
    r = rng.random()
    if r < 0.45:
        return rng.choice([1582, 1582, 1583, 1752, 1752, 1700, 1800, 1900, 1930, 1581, 2000, 2001, 1600])
    if r < 0.75:
        return rng.randint(1500, 2100)
    if r < 0.95:
        return rng.randint(-5000, 5000)
    return rng.choice([-1, 1]) * rng.choice([10 ** 6, 123456789, 10 ** 9 + 7, 4712, 4713])


def rd_month():
    return rng.choice(list(range(1, 13)) * 3 + [-1, -2, -12, -13, 0, 13, 14, -14])


def rd_day():
    return rng.choice(list(range(1, 32)) * 2 + [-1, -2, -10, -21, -25, -28, -31, -32, 0, 32])


def rd_civil():
    return f"{rd_year()} {rd_month()} {rd_day() if rng.random() < 0.5 else rng.randint(1, 28)} {rng.choice(RD_STARTS)}"


def rubydate_line():
    r = rng.random()
    if r < 0.15:
        if rng.random() < 0.3:
            return f"vc {rng.choice([1582, 1582, 1752, 1700, 1500])} {rd_month()} {rd_day()} {rng.choice(['1000', '0', '3000000', '-5', '2298873'])}"
        return f"vc {rd_civil()}"
    if r < 0.38:
        return f"cv {rd_civil()}"
    if r < 0.46:
        jd = rng.choice([rng.randint(2298000, 2427000), rng.randint(2298850, 2299200), rng.randint(-10 ** 6, 10 ** 7), rng.randint(-10 ** 11, 10 ** 11)])
        return f"jd {jd} {rng.choice(RD_STARTS)}"
    if r < 0.53:
        return f"{rng.choice(['od', 'od', 'vo'])} {rd_year()} {rng.choice([rng.randint(1, 366), rng.randint(-366, -1), 0, 367, 355, 356])} {rng.choice(RD_STARTS)}"
    if r < 0.62:
        return (f"{rng.choice(['cm', 'cm', 'vm'])} {rd_year()} {rng.choice([rng.randint(1, 52), rng.randint(1, 52), 53, rng.randint(-53, -1), 0, 54])} "
                f"{rng.choice([rng.randint(1, 7), rng.randint(1, 7), rng.randint(-7, -1), 0, 8])} {rng.choice(RD_STARTS)}")
    if r < 0.85:
        return f"{rng.choice(['sh', 'ls'])} {rd_civil()} {rng.choice([rng.randint(-30, 30), rng.randint(-1300, 1300), 1, -1, 12])}"
    if r < 0.92:
        return f"ad {rd_civil()} {rng.choice([rng.randint(-40, 40), rng.randint(-10 ** 6, 10 ** 6)])}"
    if r < 0.96:
        return f"ns {rd_civil()} {rng.choice(RD_STARTS)}"
    return f"df {rd_civil()} {rd_civil()}"


family("rubydate", "rubydate", [rubydate_line() for _ in range(int(os.environ.get("RUBYDATE_LINES", "12000")))],
       ["ruby", os.path.join(LIB, "rubydate/oracle.rb")])

# --- Ruby's Array#pack and String#unpack, against Ruby 2.6 -----------------------------------------------------------

RP_INTS = ["C", "c", "S", "s", "L", "l", "Q", "q", "J", "j", "I", "i", "n", "N", "v", "V"]
RP_COUNTS = ["", "", "", "0", "1", "2", "3", "5", "*", "*", "10"]


def rp_directive():
    r = rng.random()
    if r < 0.4:
        d = rng.choice(RP_INTS)
        if d in "sSiIlLqQjJ" and rng.random() < 0.4:
            d += rng.choice(["_", "!", "<", ">", "_<", "!>", "<_"])
        return d + rng.choice(RP_COUNTS)
    if r < 0.65:
        return rng.choice("aAZBbHh") + rng.choice(RP_COUNTS + ["7", "12"])
    if r < 0.85:
        return rng.choice(["U", "U*", "U2", "w", "w*", "w2", "u", "u*", "u3", "u70", "m", "m0", "m*", "m3", "m61", "M", "M*", "M3", "M20"])
    if r < 0.95:
        return rng.choice(["x", "x2", "X", "X2", "@", "@0", "@3", "@*", "X*", "x*", "x0"])
    return rng.choice(["y", " ", "#c\n", "a_", "C<", "s<>", "%", "S!!", "k2", "\t"])


def rp_template():
    return "".join(rp_directive() for _ in range(rng.choice([1, 1, 2, 2, 3, 4])))


def rp_int():
    return rng.choice([rng.randint(-200, 300), rng.randint(-2 ** 33, 2 ** 33), rng.randint(-2 ** 64, 2 ** 65), rng.randint(0, 0x10FFFF), rng.choice([0, -1, 255, 256,
                       65535, 2 ** 31 - 1, 2 ** 31, 0x7FFFFFFF, 2 ** 63 - 1, 2 ** 64 + 5, -(2 ** 63)])])


def rp_bytes(n):
    return bytes(rng.choice([rng.randrange(256), rng.randrange(32, 127), 0, 32, 61, 10, 0x41]) for _ in range(n))


def rp_items(template):
    """Items that mostly fit the template, sometimes too few or of the wrong kind."""
    items = []
    i = 0
    while i < len(template):
        ch = template[i]
        i += 1
        while i < len(template) and template[i] in "_!<>":
            i += 1
        count = ""
        while i < len(template) and (template[i].isdigit() or template[i] == "*"):
            count += template[i]
            i += 1
        n = 1 if count == "" else rng.randint(1, 4) if count == "*" else int(count)
        if ch in "CcSsLlQqJjIinNvVUw":
            items += [("i", rp_int()) for _ in range(n)]
        elif ch in "um" and rng.random() < 0.4:
            items.append(("s", rp_bytes(rng.randint(40, 140))))
        elif ch in "aAZumM":
            items.append(("s", rp_bytes(rng.randint(0, 12)) if ch != "M" or rng.random() < 0.5 else rng.choice([b"a b \nc=d", b"x\t\n", b"\xc3\xa9t\xe9 ", b"=" * 3])))
            if ch == "M" and rng.random() < 0.2:
                items[-1] = ("i", rp_int())
        elif ch in "Bb":
            items.append(("s", bytes(rng.choice(b"0101a ") for _ in range(rng.randint(0, 12)))))
        elif ch in "Hh":
            items.append(("s", bytes(rng.choice(b"0123456789abcdefABCDEFgz ") for _ in range(rng.randint(0, 9)))))
    if rng.random() < 0.08 and items:
        items.pop()
    if rng.random() < 0.06:
        items.insert(rng.randrange(len(items) + 1), rng.choice([("n", None), ("i", 7), ("s", b"x")]))
    return items


def rp_item_text(it):
    kind, v = it
    return "n" if kind == "n" else f"i{v}" if kind == "i" else "s" + (v.hex() or "-")


def rp_data():
    r = rng.random()
    if r < 0.4:
        return rp_bytes(rng.randint(0, 20))
    if r < 0.55:
        return rng.choice([b"QUJD", b"QQ==", b"QUI=", b"QUJDRA", b"QQ", b"Q===", b"QUJ\nDRA==\n", b"#QU\x00JD", b"QR==", b"QUJ=", b"=QUJD", b""]) + rp_bytes(rng.randint(0, 3))
    if r < 0.7:
        return rng.choice([b"a=3Db", b"=41=\n=42", b"x=\r\ny", b"=XY", b"=4", b"=", b"caf=C3=A9", b"a \nb"]) + rp_bytes(rng.randint(0, 3))
    if r < 0.8:
        return rng.choice([b"#86)C\n", b"!80``\n", b"$86)C\n", b"\"86)C", b"`\n", b"(;F]R;&0`"]) + rp_bytes(rng.randint(0, 2))
    return rng.choice(["héllo", "😀a", "߿", "x"]).encode() + rng.choice([b"", b"\xc3", b"\xfe", b"\xc0\x80", b"\xe0\x80\x80", b"\xfc\x84\x80\x80\x80\x80",
                                                                           b"\x80", b"\xf8\x88\x80\x80\x80"])


def rubypack_line():
    h = lambda b: b.hex() or "-"
    t = rp_template()
    if rng.random() < 0.45:
        items = rp_items(t)
        return f"pk {h(t.encode())} {'.'.join(rp_item_text(it) for it in items) if items else '~'}"
    if rng.random() < 0.2:
        t = rng.choice(["m0", "m0a*", "m", "ma*", "M", "Ma*", "u", "ua*", "U*", "w*", "@", "a@", "C@", "x@", "@a", "m0C"])
        data = rp_data() if rng.random() < 0.7 else rng.choice([b"", b"a", b"QQ==QQ==", b"QUJD=", b"QQ==\n"])
        return f"{rng.choice(['up', 'up', 'u1'])} {h(data)} {h(t.encode())}"
    return f"{rng.choice(['up', 'up', 'up', 'u1'])} {h(rp_data())} {h(t.encode())}"


family("rubypack", "rubypack", [rubypack_line() for _ in range(int(os.environ.get("RUBYPACK_LINES", "12000")))],
       ["ruby", os.path.join(LIB, "rubypack/oracle.rb")])

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
