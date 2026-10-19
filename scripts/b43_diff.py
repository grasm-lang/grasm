"""URLs, domains, form data, MIME types, data: URLs and package specifiers,
each against the implementation everyone else runs.

- WHATWG URLs (parsing, every getter, the setters), domain to ASCII and
  Unicode (UTS #46), application/x-www-form-urlencoded and
  URLSearchParams, MIME types and data: URLs against Node 25 (ada 3.3,
  util.MIMEType, fetch).
- hosted-git-info, node-semver, npm-package-arg and negotiator against
  the copies inside npm itself.
- Python wheel tags against pip's vendored packaging.

Node has a few bugs a standard-conforming implementation must not copy,
so inputs that reach them are left out (and counted). Its URL parser,
against a base with an opaque path, accepts any input holding a `#` (the
standard accepts one only when it starts with `#`), and when shortening a
file path keeps a first segment that merely starts with a drive letter;
two setter cases are left out for the same reason (WPT expects what is
computed here: an empty host on a URL without one, and a host given to a
URL whose path starts with an empty segment). util.MIMEType does not trim
trailing white space from its input, which shows in a quoted parameter
value left open to the end.
"""
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b43")
harness = os.path.join(work, "b43test")
built = subprocess.run([GRASM, "build", "packages/b43test/src/main.gr", "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
ORACLE = os.path.join(ROOT, "scripts/lib/b43/oracle.mjs")
TAGS = os.path.join(ROOT, "scripts/lib/b43/tags.py")
cases = []
counter = [0]
skipped = {}


def write(text, ext="txt"):
    counter[0] += 1
    p = os.path.join(work, f"f{counter[0]}.{ext}")
    open(p, "w").write(text)
    return p


def hx(s):
    return s.encode("utf-8").hex() or "_"


def un(h):
    return "" if h in ("-", "_") else bytes.fromhex(h).decode("utf-8", "replace")


def oracle(op, lines):
    p = write("\n".join(lines) + "\n")
    if op == "tags":
        cmd = ["python3.13", TAGS, p]
    else:
        cmd = ["node", ORACLE, op, p]
    r = subprocess.run(cmd, capture_output=True, text=True, env=dict(os.environ, HOME="/home/u"), timeout=1800)
    if r.returncode != 0:
        raise RuntimeError(r.stderr[-800:])
    out = r.stdout.split("\n")[:-1]
    assert len(out) == len(lines), (op, len(out), len(lines))
    return out


def add_family(fam, op, lines, n_cases, keep=lambda line, answer: True):
    answers = oracle(op, lines)
    pairs = []
    for line, a in zip(lines, answers):
        if keep(line, a):
            pairs.append((line, a))
        else:
            skipped[fam] = skipped.get(fam, 0) + 1
    size = max(1, len(pairs) // n_cases)
    for k in range(n_cases):
        chunk = pairs[k * size:(k + 1) * size] if k < n_cases - 1 else pairs[k * size:]
        if not chunk:
            continue
        want = [a for _, a in chunk]
        p = write("\n".join(l for l, _ in chunk) + "\n")

        def check(got, want=want, chunk=chunk):
            mine = got.split(";")
            if len(mine) != len(want):
                return False, f"{len(want)} answers", f"{len(mine)} answers"
            for (l, _), m, w in zip(chunk, mine, want):
                if m != w:
                    return False, f"{l[:80]} -> {w[:150]}", m[:150]
            return True, "", ""
        cases.append((op, p, check, fam))


# --- URLs ----------------------------------------------------------------------------

SCHEMES = ["http", "HTTPS", "file", "ws", "wss", "ftp", "sc", "mailto", "data", "blob", "javascript", "a1+.-", "", "1x", "FILE"]
SEPS = ["://", ":", ":/", ":\\\\", ":///", ":\\", "://\\", ":////"]
USER = ["", "", "user:pass@", "@", "a@b@", "u:@", ":p@", "us er:p%41ss@", "é:ü@", "x:y:z@"]
HOSTS = ["example.com", "EXAMPLE.com", "127.0.0.1", "0x7f.1", "1.2.3", "[::1]", "[1:2::3:4]", "[::ffff:1.2.3.4]", "münchen.de", "xn--mnchen-3ya.de",
         "％４１.com", "%41.com", "a%2eb", "localhost", "", "0177.0.0.1", "4294967295", "4294967296", "1.2.3.4.5", "[0:0:0:0:0:0:0:0]", "[::]", "[1::]",
         "[1:0:0:2::3]", "[::1.2.3.256]", "a b", "a<b", "faß.de", "ＥＸＡＭＰＬＥ。com", "xn--a", "١٢٣", "a.b.c.", "0x", "09.1", "08", "[v1.x]", "h\u0000x",
         "%zz", "%e2%82", "x.0x.1", "[1:2:3:4:5:6:7:8]", "[::127.0.0.1]", "0X7F.0.0.1", "1.0x2", "999.1", "a..b"]
PORTS = ["", "", ":80", ":443", ":0", ":65535", ":65536", ":8080", ":", ":1a", ":00021"]
PATHS = ["", "/", "/a/b", "/./x", "/../x", "/%2e%2E/x", "/a/.%2e/b", "\\x\\y", "/C:/x", "/c|/x", "/a b/c", "/é", "/^`{}", "/a//b", "/..", "/.", "/%2e",
         "C|", "/x?", "/x#", "x/y", "/ü%FF"]
QUERY = ["", "", "?a=b c", "?é", "?'", "?a#?", "?%zz", "?\"<>"]
FRAG = ["", "", "#x y", "#é", "#`", "#a#b", "#%41"]
REL = ["", "?q", "#f", "//host/p", "/abs", "rel/x", "../up", ".", "..", "C:/x", "\\\\server\\share", "//", "///x", "?", "#", "a:b", "//[::1]:9/",
       "x\t\ny", "  /sp  ", "\\a\\..\\b", "file:x", "http:y", "/C|/w", "..\\..\\z"]
BASES = ["http://example.com/a/b?c#d", "file:///C:/dir/file", "sc://h/x/y", "mailto:x", "about:blank", "http://[::1]:8080/", "file://host/share/f",
         "sc:/p/q", "https://u:p@x.org:99/d/e/f", "sc:x"]
OPAQUE_BASES = {"mailto:x", "about:blank", "sc:x"}


def url_line():
    if rng.random() < 0.65:
        s = rng.choice(SCHEMES) + rng.choice(SEPS) + rng.choice(USER) + rng.choice(HOSTS) + rng.choice(PORTS) + rng.choice(PATHS) + rng.choice(QUERY) + \
            rng.choice(FRAG)
        if rng.random() < 0.2:
            k = rng.randrange(len(s) + 1)
            s = s[:k] + rng.choice(["\t", "\n", " ", "\r", "\x01"]) + s[k:]
        base = rng.choice(BASES) if rng.random() < 0.3 else None
    else:
        s = rng.choice(REL) + (rng.choice(PATHS) if rng.random() < 0.3 else "") + rng.choice(QUERY) + rng.choice(FRAG)
        base = rng.choice(BASES)
    if rng.random() < 0.1:
        s = " " + s + "\x1f "
    return hx(s) + " " + (hx(base) if base else "-")


def node_url_bug(line, answer):
    a, b = line.split(" ")
    inp, base = un(a), (un(b) if b != "-" else None)
    s = re.sub(r"[\t\n\r]", "", inp).strip("".join(chr(c) for c in range(33)))
    if base in OPAQUE_BASES and not re.match(r"^[A-Za-z][A-Za-z0-9+.\-]*:", s) and "#" in s[1:] and not s.startswith("#"):
        return True
    if answer != "FAIL":
        f = answer.split(",")
        if un(f[2]) == "file:" and re.match(r"^/[A-Za-z][:|][^/]", un(f[8])) and re.search(r"\.\.|%2e", s, re.I):
            return True
    return False


add_family("url-parse", "url", [url_line() for _ in range(1600)], 16, keep=lambda l, a: not node_url_bug(l, a))

SET_URLS = ["http://example.com/a/b?c#d", "https://u:p@x.org:8443/d", "file:///C:/dir/file", "file://host/share", "sc://h:9/x/y?q", "sc:/p", "sc:opaque path  ",
            "mailto:x@y", "ws://[::1]:80/", "ftp://f/", "http://h/", "sc://", "data:text/plain,hi ", "javascript:alert(1)", "blob:https://a/b", "non-spec:/.//p"]
SETTERS = {
    "href": ["http://n/", "bad", "", "file:///x", "sc:y"],
    "protocol": ["https", "HTTP:", "file", "sc", "mailto:", "ws", "1x", "a+b", "http:x", "ftp"],
    "username": ["", "a b", "u:v@w", "é", "%41"], "password": ["", "p:q", "ü", "a/b"],
    "host": ["new.org", "new.org:81", "[::2]:3", "", "a b", "h:", "h:99999", "münchen.de", "x/y", "x?y", "1.2.3", "h:80"],
    "hostname": ["new.org", "h:81", "", "[::2]", "Ex.COM", "a#b", "localhost"],
    "port": ["", "81", "443", "80", "abc", "8a", "99999", "0", "21x"],
    "pathname": ["/p", "q", "", "/a b/../c", "\\x\\y", "/%2e/", "C:/w", "?#"],
    "search": ["", "?", "a=b", "?é c", "#x", "??"], "hash": ["", "#", "x y", "#é", "##"],
}


def setter_line():
    st = rng.choice(list(SETTERS))
    return " ".join(hx(x) for x in (rng.choice(SET_URLS), st, rng.choice(SETTERS[st])))


def node_setter_bug(line, answer):
    u, s, v = (un(x) for x in line.split(" "))
    if s in ("host", "hostname") and u in ("sc:/p", "non-spec:/.//p", "sc:opaque path  ", "mailto:x@y", "javascript:alert(1)", "data:text/plain,hi "):
        return v == "" or u == "non-spec:/.//p"
    return False


add_family("url-setters", "set", [setter_line() for _ in range(1200)], 12, keep=lambda l, a: not node_setter_bug(l, a))

DOMAIN_PIECES = ["a", "B", "z9", "-", ".", "xn--", "ß", "ς", "Σ", "ﬁ", "Ａ", "。", "\u00ad", "\u200c", "\u200d", "\u0628", "\u0627", "\u0915\u094d", "é", "e\u0301",
                 "\u0301", "日本", "☃", "💩", "Ⅳ", "ı", "İ", "\uff0e", "xn--ls8h", "xn--a", "xn--mnchen-3ya", "Ö", "\u00bd", "\u0640", "\u05d0", "1", "\ufe0f", " ",
                 "%", "_", "\U0001d400", "\u1e9e", "\u212b", "0x7f", "[::1]", "/", "?x", "#", ":8", "[1::"]
add_family("uts46", "domain", [hx("".join(rng.choice(DOMAIN_PIECES) for _ in range(rng.randrange(1, 8)))) for _ in range(800)], 10)

# --- forms, MIME types, data: URLs ----------------------------------------------------------

QS = ["a=b", "a", "=b", "", "a=b=c", "+", "a+b=c+d", "%41", "%zz", "%e2%82", "é=ü", "a&&b", "?x=1", "💩=1", "\ufffd=2", "%F0%9F%92%A9=3", "b=1", "a=2",
      "*-._=~!'()", "x y=z", "%2B", "\u00e9=\u0301"]
add_family("form", "qs", [hx("&".join(rng.choice(QS) for _ in range(rng.randrange(0, 7)))) for _ in range(500)], 8)
MIME = ["text/html", "TEXT/HTML;Charset=UTF-8", " text/plain ; a=b ", "a/b;c=\"d\\\"e\";c=f", "x/y;=v", "x/y;n=", "x/y;n=\"\"", "a/", "/b", "a b/c", "a/b c",
        "x/y; n = v", "x/y;n=v w", "x/y;N=\"q\\\\\"", "x/y;n=\"unterminated", "x/y;;n=1", "x/y;é=1", "x/y;n=é", "x/y;n=\x7f", "application/vnd.a+json",
        "text/html;charset=\"utf-8\";charset=latin1", "x/y;n=\"a;b\""]


def node_mime_bug(line, answer):
    # util.MIMEType never trims trailing white space from the whole input
    # (the standard's first step), which shows when a quoted value is left
    # open to the end.
    s = un(line).lstrip("\t\n\r ")
    inside, escaped = False, False
    for c in s:
        if inside:
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == '"':
                inside = False
        elif c == '"':
            inside = True
    return inside and s != s.rstrip("\t\n\r ")


add_family("mime", "mime", [hx(rng.choice(MIME) + (rng.choice(MIME)[3:] if rng.random() < 0.3 else "")) for _ in range(500)], 8,
           keep=lambda l, a: not node_mime_bug(l, a))
DATA = ["data:,hi", "data:text/plain,a%20b", "data:;base64,aGk=", "data:text/html;base64, aG k=", "data:;BASE64,aGk", "data:text/plain ; base64,aGk=",
        "data:x/y;base64,a===", "data:x/y;base64,a", "data:,#frag", "data:%,x", "data:text/plain;charset=utf-8,é", "data:a/b;base64x,1", "data:bad,1",
        "data:;charset=x,1", "data: text/plain ,x", "data:text/plain;base64;x,aGk=", "data:\t,a\nb", "data:x/y;base64,////", "data:,%e2%82",
        "data:a b/c;base64,aGk=", "DATA:,x", "data:x/y;base64,=aGk", "data:x/y;base64,aGk=;"]
add_family("dataurl", "data", [hx(rng.choice(DATA)) for _ in range(800)], 8)

# --- npm ------------------------------------------------------------------------------------

GH_HOSTS = ["github.com", "www.github.com", "gitlab.com", "bitbucket.org", "gist.github.com", "git.sr.ht", "example.com", "GitHub.com"]
PROTOS = ["https://", "http://", "git://", "git+ssh://git@", "git+https://", "ssh://git@", "git+http://", "", "git@", "github:", "gitlab:", "bitbucket:", "gist:",
          "sourcehut:", "ftp://"]
GUSERS = ["npm", "u-1", "", "a%20b", "user name", "grp/sub", "%zz", "~u"]
GPROJS = ["cli", "cli.git", "repo-x", "p%2F", "", "a.b", "x%E2%82"]
GTAILS = ["", "#main", "#v1.2.3", "#semver:^1.0.0", "#path:sub/dir", "/tree/main", "/tree", "/tree/main/sub", "/-/tree/main", "/archive.tar.gz", "/get/x",
          "/raw", "/blob/x", "#a%20b", "#%zz", "/archive/x"]
GAUTH = ["", "", "user:pass@", "tok@", ":pw@"]


def hosted_input():
    k = rng.random()
    if k < 0.15:
        return rng.choice(GUSERS) + "/" + rng.choice(GPROJS) + rng.choice(["", "#x", "#semver:~2", " #y", "/"])
    if k < 0.3:
        return "git@" + rng.choice(GH_HOSTS) + ":" + rng.choice(GUSERS) + "/" + rng.choice(GPROJS) + rng.choice(["", "#main"])
    p = rng.choice(PROTOS)
    host = "" if p.endswith(":") and not p.endswith("://") and p != "git@" else rng.choice(GH_HOSTS)
    sep = ":" if p == "git@" else "/"
    return p + (rng.choice(GAUTH) if "://" in p and "@" not in p else "") + host + (sep if host else "") + rng.choice(GUSERS) + "/" + rng.choice(GPROJS) + \
        rng.choice(GTAILS)


add_family("hostedgit", "hosted", [hx(hosted_input()) for _ in range(1200)], 12)


def sv_num():
    return rng.choice(["0", "1", "2", "10", "01", "00", "9007199254740991", "9007199254740992", "x", "X", "*", "3"])


def sv_version():
    v = rng.choice(["", "v", "=", "v=", " v", "= "]) + ".".join(sv_num() for _ in range(rng.choice([3, 3, 3, 2, 1, 4]))) + \
        rng.choice(["", "", "-alpha", "-beta.1", "-0", "-rc.1.2", "-01", "-alpha-1", "-1.2", "-x.7.z.92", "alpha", "-a.b.c", "-9007199254740992"]) + \
        rng.choice(["", "", "+build", "+001", "+a.b", "+-x"])
    return v if rng.random() < 0.9 else " " + v + "  "


def sv_range():
    k = rng.random()
    if k < 0.15:
        return sv_version().strip() + " - " + sv_version().strip()
    if k < 0.2:
        return rng.choice(["*", "", "x", "latest", ">=0.0.0", ">=0.0.0-0", "<0.0.0-0", "1.x || >=2.5.0 || 5.0.0 - 7.2.3", "^1.2.3+build", ">1 <0"])
    comp = lambda: rng.choice(["", "", ">", ">=", "<", "<=", "=", "~", "^", "~>", "> ", "^ ", "~ "]) + sv_version().strip()
    return " || ".join(" ".join(comp() for _ in range(rng.randrange(1, 3))) for _ in range(rng.randrange(1, 3)))


add_family("nodesemver", "semver", [hx(sv_version()) + " " + hx(sv_range()) + " " + ",".join(hx(sv_version().strip()) for _ in range(5)) for _ in range(800)], 8)
NAMES = ["lodash", "@scope/pkg", "@s/p", "React", "a.b", "_x", ".x", "node_modules", "fs", "my-pkg", "@scope", "x y", "é", "@a/b/c", "foo~1"]
SPECS = ["", "*", "1.2.3", "^1.2", "~1.2.3", ">=1 <2", "latest", "next", "v1.2.3", "=1.2.3", "1.2.3-beta", "1.x || 2", "tag with space", "%41", "npm:react@18",
         "npm:@s/p@^1", "npm:npm:x", "npm:github:a/b", "github:user/repo", "github:user/repo#v1", "user/repo#semver:^2", "gitlab:g/p",
         "git+ssh://git@github.com:npm/cli.git#path:packages/a", "git+ssh://git@host.com:repo.git#main", "git+ssh://git@host.com:22/repo.git",
         "git://host.com/x.git#semver:1.x::path:sub", "git+https://host.com/x.git#a::b", "git+file:///tmp/r.git#c", "git+file://C:/repo",
         "https://x.org/y.tgz", "http://x/y", "ftp://x/y", "file:../x.tgz", "file:./dir", "./local", "../up/x.tar.gz", "~/code/pkg", "/abs/path",
         "file:///abs/x", "file://host/x", "file:~/z", "C:/win", "x.tgz", "dir/sub", "file:%zz", "file:a b", "bitbucket:u/p", "gist:abc",
         "git@github.com:npm/cli.git", "git@gitlab.com:g/p.git#semver:~1", "user@host.example:repo", "git+ssh://x:y#semver:%zz"]


def npa_input():
    k = rng.random()
    return rng.choice(NAMES) + "@" + rng.choice(SPECS) if k < 0.45 else rng.choice(SPECS) if k < 0.85 else rng.choice(NAMES)


add_family("npmspec", "npa", [hx(npa_input()) for _ in range(800)], 8)
NQ = ["", "", ";q=0.5", ";q=1", ";q=0", ";q=.3x", ";q=abc", ";q=1e-1", ";q=Infinity", ";q= 0.7", ";q=-1", ";Q=0.2", ";q=0.001", ";q"]
NPOOL = {"m": ["text/html", "text/*", "*/*", "application/json", "TEXT/HTML", "text/plain;charset=utf-8", "text/plain;charset=\"UTF-8\"",
               "text/plain;format=flowed", "a/b;x=\"1,2\"", "image/png", "bad", "/x", "text/", "text/html junk", "x/y;level=1", "x/y;level=*", "*/json"],
         "c": ["utf-8", "UTF-8", "iso-8859-1", "*", "latin1", "x y", ";q=1", "utf-16le"],
         "e": ["gzip", "deflate", "br", "identity", "*", "GZIP", "compress", "x;q=0"],
         "l": ["en", "en-US", "en-us", "fr", "fr-CA", "*", "zh-Hant-TW", "en-", "-x", "de-DE-1996", "x y"]}


def neg_input():
    k = rng.choice("mcel")
    pool = NPOOL[k]
    hdr = "-" if rng.random() < 0.1 else hx(",".join(rng.choice(pool) + rng.choice(NQ) for _ in range(rng.randrange(0, 5))))
    offers = "-" if rng.random() < 0.3 else (",".join(hx(x) for x in rng.sample(pool, rng.randrange(0, min(6, len(pool))))) or "_")
    pref = ",".join(hx(x) for x in rng.sample(NPOOL["e"], 3)) if k == "e" and rng.random() < 0.4 else "-"
    return f"{k} {hdr} {offers} {pref}"


add_family("negotiator", "neg", [neg_input() for _ in range(1200)], 12)

# --- Python tags ----------------------------------------------------------------------------

PLAT = ["macosx_11_0_arm64", "manylinux_2_17_x86_64", "win_amd64", "linux_x86_64", "any", "MacOSX_10_9_X86_64", "musllinux_1_2_aarch64"]
ABI = ["cp312", "cp313t", "cp37m", "cp27mu", "abi3", "none", "cp39d", "pypy39_pp73", "CP311"]
WHEELS = ["py3-none-any", "cp312-cp312-macosx_11_0_arm64", "cp39.cp310-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64", "py2.py3-none-any",
          "cp311-abi3-win_amd64", "cp312-none-any", "pp39-pypy39_pp73-any"]


def tag_input():
    k = rng.choice(["cp", "gen", "compat", "mac", "parse", "rank"])
    v = rng.choice(["3.12", "3.2", "3.1", "2.7", "3", "3.13", "4.0", "3.10.1", "3.0"])
    plats = ",".join(rng.sample(PLAT, rng.randrange(1, 4)))
    abis = ",".join(rng.sample(ABI, rng.randrange(0, 4))) or "_"
    if k == "cp":
        return f"cp {v} {abis} {plats}"
    if k == "gen":
        return f"gen {rng.choice(['pp39', 'cp312', 'GraalPy', 'ip27'])} {abis} {plats}"
    if k == "compat":
        return f"compat {v} {rng.choice(['_', 'cp312', 'pp310'])} {plats}"
    if k == "mac":
        return f"mac {rng.choice(['10.0', '10.3', '10.4', '10.6', '10.15', '11.0', '12.4', '15.1', '26.0'])} " + \
            rng.choice(['arm64', 'x86_64', 'i386', 'ppc', 'ppc64', 'intel', 'universal2'])
    if k == "rank":
        return f"rank {rng.choice(['3.12', '3.9', '3.11'])} {abis} {plats} " + ",".join(hx(w) for w in rng.sample(WHEELS, rng.randrange(1, 3)))
    return "parse " + hx(rng.choice(WHEELS + ["Py2.PY3-NONE-ANY", "a-b", "a-b-c-d", "x..y-z-w"]))


add_family("pytags", "tags", [tag_input() for _ in range(800)], 8)


# --- run ----------------------------------------------------------------------------------------

def finish():
    answers = []
    for i in range(0, len(cases), 100):
        argv = []
        for op, path, _, _ in cases[i:i + 100]:
            argv += [op, path]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
        if ran.returncode != 0:
            for op, path, _, _ in cases[i:i + 100]:
                one = subprocess.run([harness, op, path], capture_output=True, text=True)
                if one.returncode != 0:
                    print(f"harness crashed on {op} {path}")
                    break
            print(ran.stderr[-2000:]); sys.exit(1)
        answers.extend(ran.stdout.split("\n")[:-1])
    if len(answers) != len(cases):
        print(f"{len(answers)} answers for {len(cases)} cases"); sys.exit(1)
    families, differ = {}, 0
    only = os.environ.get("ONLY", "")
    for (op, path, check, fam), got in zip(cases, answers):
        try:
            ok, want, mine = check(got)
        except Exception as e:
            ok, want, mine = False, "an answer the reference can read", f"{type(e).__name__}: {str(e)[:200]}"
        f = families.setdefault(fam, [0, 0])
        if ok:
            f[0] += 1
        else:
            f[1] += 1
            differ += 1
            if fam.startswith(only) and (differ <= 8 or (only and f[1] <= 8)):
                print(f"  {fam} {op} {path}")
                print(f"    want  {str(want)[:300]}")
                print(f"    grasm {str(mine)[:300]}")
    bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
    if bad:
        print("disagreeing:", " ".join(bad), f"(SEED={SEED})")
    print("families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
    print("left out for Node's bugs: " + (" ".join(f"{k}={v}" for k, v in sorted(skipped.items())) or "none"))
    thin = [k for k, (a, d) in families.items() if a + d < 8]
    if thin or len(families) < int(os.environ.get("FAMILIES", "11")):
        print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
    print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0)


finish()
