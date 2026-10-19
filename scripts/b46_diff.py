"""npm's and pip's own readings of what packages say, against the code
that reads them here.

- Brace expansion against brace-expansion 2.0.2.
- npm's INI (`.npmrc`) read, written and quoted against ini 5.0.0.
- License texts corrected against spdx-correct 3.2.0, and expressions
  parsed against the spdx-expression-parse it carries.
- Engines, platforms and devEngines against npm-install-checks 7.1.2.
- Versions picked from packuments against npm-pick-manifest 11.0.1 (run
  with TZ=UTC: a time without an offset is read as UTC here).
- Requirements files, nested, against pip 24.3's own parser.
- entry_points.txt against Python 3.13's importlib.metadata.

The tables spdxcorrect and pyentrypoints carry (license and exception
identifiers, spdx-correct's sorted rewrite lists, Python's `\\w`) are
checked against their sources first, so a newer npm or Python makes the
gate fail rather than drift.
"""
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
SEED = int(os.environ.get("SEED", random.randrange(1 << 30)))
rng = random.Random(SEED)
work = tempfile.mkdtemp(prefix="b46")
NPM = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() + "/npm/node_modules"
LIB = os.path.join(ROOT, "scripts/lib/b46")
harness = os.path.join(work, "b46test")
built = subprocess.run([GRASM, "build", "packages/b46test/src/main.gr", "--native", "-o", harness], cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

# --- the tables, against their sources ------------------------------------------------------------

stale = []
spdx_src = open(os.path.join(ROOT, "packages/spdxcorrect/src/main.gr")).read()
for line in subprocess.run(["node", os.path.join(LIB, "spdx_tables.js"), NPM], capture_output=True, text=True).stdout.splitlines():
    if line not in spdx_src:
        stale.append("packages/spdxcorrect: " + line[:60])
word = subprocess.run(["python3.13", os.path.join(LIB, "pyword.py")], capture_output=True, text=True).stdout.strip()
if word not in open(os.path.join(ROOT, "packages/pyentrypoints/src/main.gr")).read():
    stale.append("packages/pyentrypoints: PY_WORD")
if stale:
    print("tables out of date with their sources (regenerate with scripts/lib/b46/):")
    for s in stale:
        print("  " + s)
    sys.exit(1)

cases = []   # (family, op, path, inputs)
hx = lambda s: s.encode().hex() or "-"
o_ = lambda x: "_" if x is None else hx(x)


def case_file(lines):
    p = os.path.join(work, f"c{len(cases)}.txt")
    open(p, "w", encoding="utf-8").write("\n".join(lines) + "\n")
    return p


# --- brace expansion -----------------------------------------------------------------------------------

PIECES = ["{", "}", ",", "a", "b", "c", "..", "1", "2", "10", "-3", "05", "x", "Z", "\\", "\\{", "\\}", "\\,", "\\.", "\\\\", "$", "${", "{}", "{a,b}", "{1..3}",
          "{a..c}", "{0..10..3}", "{05..1}", "{a..e..2}", "{z..a}", "é", "\n", " ", ".", "{X..b}"]


def brace_pattern():
    parts, sets = [], 0
    while len(parts) < rng.randrange(0, 18):
        x = rng.choice(PIECES)
        if x.startswith("{") and len(x) > 2:
            if sets >= 2:
                continue
            sets += 1
        parts.append(x)
    return "".join(parts)


for _ in range(10):
    lines = [hx(brace_pattern()) for _ in range(150)]
    cases.append(("brace-expansion", "brace", case_file(lines), lines))

# --- npm's INI -----------------------------------------------------------------------------------------

KEYS = ["a", "b", "c", "a.b", "x y", "0", "1", "10", "02", "length", "__proto__", "toString", "constructor", "'1'", "'[1,2]'", "'{\"k\":1}'", "\"q\"", "'true'",
        "\"__proto__\"", "k[]", "a[]", "[]", "x[]", "'null'", "é", "\U0001F600", "a\\;b", "a\\.b", " sp ", "'1.50'", "'-0'", "'1e400'", "' 7 '", "''", "\"\""]
VALS = ["1", "x", "true", "false", "null", "'1'", "'true'", "\"s\"", "'{\"a\":[1,2]}'", "'[1,{\"b\":null}]'", "a ; c", "a # c", "a\\;b", "a\\#b", "a\\\\b", "a\\x",
        " spaced ", "'x'", "\"bad", "'1e400'", "'-0'", "\"\\u00e9\\n\"", "[x]", "=", "a=b", "é", "\U0001F600", "'\"q\"'", "\u2028x", "'0.1'", "'[]'", "'{}'"]
SECTIONS = ["[a]", "[a.b]", "[a\\.b]", "[a.b.c]", "[b]", "[ x ]", "[__proto__]", "['1']", "['[1,2]']", "[\"q\"]", "[0]", "[length]", "[a]  ", "[a]x", "[k]", "[x.y]",
            "[toString]", "['null']", "[]", "[a..b]", "[.a]", "[a.]"]


def ini_text():
    out = []
    for _ in range(rng.randrange(0, 14)):
        r = rng.random()
        if r < 0.2:
            out.append(rng.choice(SECTIONS))
        elif r < 0.8:
            k = rng.choice(KEYS)
            out.append(k if rng.random() < 0.1 else k + rng.choice(["=", " = ", "= ", " ="]) + rng.choice(VALS))
        else:
            out.append(rng.choice(["# c", "; c", "", "   ", "=x", "\t; x"]))
    return "".join(l + rng.choice(["\n", "\r\n", "\r", "\n\n"]) for l in out)


def jvalue(d=0):
    r = rng.random()
    if d > 2 or r < 0.5:
        return rng.choice(["x", "a=b", "[y", " s", "q;#", "\"q\"", "'z'", 1, 1.5, -0.0, 1e21, True, False, None, "", "é", "\U0001F600", "\n"])
    if r < 0.75:
        return [jvalue(d + 1) for _ in range(rng.randrange(0, 4))]
    return {rng.choice(KEYS + ["a", "b", "c"]): jvalue(d + 1) for _ in range(rng.randrange(0, 5))}


for _ in range(10):
    lines = [f"{rng.choice(['dec', 'decd'])} {hx(ini_text())}" for _ in range(60)]
    cases.append(("ini-decode", "ini", case_file(lines), lines))
for _ in range(10):
    lines = []
    for _ in range(60):
        obj = {rng.choice(KEYS + ["a", "b"]): jvalue() for _ in range(rng.randrange(0, 7))}
        sec = rng.choice(["-", "-", "s", "a.b", "x y", "[z"])
        o = ",".join([sec.encode().hex() if sec != "-" else "-"] + [str(rng.randrange(2)) for _ in range(6)])
        lines.append(f"enc {hx(json.dumps(obj))} {o}")
    cases.append(("ini-encode", "ini", case_file(lines), lines))
for _ in range(8):
    lines = [f"{rng.choice(['safe', 'unsafe'])} {hx(rng.choice(VALS + KEYS))}" for _ in range(80)]
    cases.append(("ini-quoting", "ini", case_file(lines), lines))

# --- spdx-correct ---------------------------------------------------------------------------------------

BASE = ["MIT", "mit", "MIT License", "M.I.T.", "Apache 2.0", "Apache-2", "Apache Version 2", "Apache License, Version 2.0", "apache2", "GPL", "GPLv2", "GPL v3", "GPL-2.0",
        "GPL 3", "GPL-3.0+", "LGPLv2.1", "LGPL 2.1+", "GNU GPL v3", "GNU Lesser General Public License", "GNU LESSER GENERAL PUBLIC LICENSE v2.1", "AGPL", "Affero",
        "BSD", "BSD 3", "BSD-3", "BSD 2 clause", "BSD clause 3", "New BSD license", "Simplified BSD", "FreeBSD", "Net BSD License", "Clear BSD", "Old BSD", "Original BSD",
        "ISC", "ISD", "IST", "MPL 2.0", "MPL/2.0", "Mozilla Public License 2.0", "CC BY 4.0", "CC-BY-SA-4.0", "BY-NC", "BY-NC-4.0", "Attribution-NonCommercial 4.0 International",
        "Attribution-NoDerivatives", "WTFPL", "WTF", "Unlicense", "UNLICENSED", "Public Domain", "Zlib", "ZLIB", "zlib", "Boost", "BSL", "Artistic 2", "Eclipse", "CDDL",
        "MIT AND Apache-2.0", "(MIT OR Apache-2.0)", "GPL-2.0-only WITH Classpath-exception-2.0", "LicenseRef-foo", "DocumentRef-x:LicenseRef-y", "MIT +", "MIT+",
        "Apache-2.0+", "()", "MIT AND", "AND", "#MIT", "MIT #", "x MIT", "+", "-", "  ", "GPLv2+", "LGPL-3.0", "MPL", "Beerware", "beer", "X11", "AFL", "EUPL", "OSL",
        "Python-2.0", "PSF", "CC0", "cc0-1.0", "0BSD", "MITNFA", "MIT +no-false-attribs", "Ruby", "JSON", "Unicode", "Sendmail", "GLP", "Gnu public license", "MTI", "-License",
        "MIT-License", "Apache-License", "Universal Permissive License", "WTH", "BSD-3-Cla" "ude", "GNU/GPL", "GUN", "APGL", "Gpl", "foo", "é", "Ⅻ", "GPL-2.0 OR MIT", "MIT WITH x"]


IDS = json.load(open(os.path.join(NPM, "spdx-license-ids/index.json")))


def mutated_id():
    # A real identifier written the ways people get them wrong.
    i = rng.choice(IDS)
    k = rng.randrange(8)
    if k == 0:
        return i.replace("-", "v", 1)
    if k == 1:
        return i.replace("-", " ")
    if k == 2:
        return i.replace("-", "")
    if k == 3:
        return i.lower()
    if k == 4:
        return i + " License"
    if k == 5:
        return i.replace(".", "")
    if k == 6:
        return i.replace(".0", "")
    return i.replace("-", " v")


def license_text():
    if rng.random() < 0.35:
        return mutated_id()
    s = rng.choice(BASE)
    if rng.random() < 0.4:
        return s
    for _ in range(rng.randrange(0, 3)):
        k = rng.randrange(6)
        if k == 0 and s:
            i = rng.randrange(len(s)); s = s[:i] + s[i + 1:]
        elif k == 1:
            i = rng.randrange(len(s) + 1); s = s[:i] + rng.choice(" .-+,/()vV0123#:é") + s[i:]
        elif k == 2:
            s = s.upper()
        elif k == 3:
            s = s.lower()
        elif k == 4:
            s = rng.choice(BASE) + rng.choice([" ", " AND ", " OR ", " WITH ", "/", ","]) + s
        else:
            s = " " + s + " "
    return s


for _ in range(8):
    lines = [f"valid {hx(license_text())}" for _ in range(120)]
    cases.append(("spdx-parse", "spdx", case_file(lines), lines))
for _ in range(10):
    lines = [f"{rng.choice(['fix', 'keep'])} {hx(license_text())}" for _ in range(100)]
    cases.append(("spdx-correct", "spdx", case_file(lines), lines))

# --- npm-install-checks -----------------------------------------------------------------------------------

RANGES = [">=14", "^18.0.0", "<16", "18.x || 20.x", ">=1.0.0-alpha", "*", "", "x", "not a range", "1.2.3", ">=22.0.0-0", "~8.1", ">= 1 <2", "^0.0.1"]
VERS = ["18.17.0", "20.0.0", "22.1.0-beta.1", "1.2.3", "8.1.4", "v18.0.0", "16.0.0", "", "nope", "0.0.1", "10.9.0", "22.0.0-rc.1"]


def jv(pool):
    return rng.choice(pool) if rng.random() < 0.75 else rng.choice([1, 0, True, False, None, [], {}, ["x"], {"a": 1}])


def engines():
    t = {}
    if rng.random() < 0.9:
        t["engines"] = rng.choice([{}, None, "x", 0, [], {"node": jv(RANGES)}, {"npm": jv(RANGES)}, {"node": jv(RANGES), "npm": jv(RANGES)}])
    return t


OSES = ["darwin", "linux", "win32", "freebsd", "!darwin", "!linux", "!win32", "any", "aix", ""]


def plat_list():
    r = rng.random()
    if r < 0.4:
        return rng.choice(OSES)
    if r < 0.85:
        return [rng.choice(OSES) for _ in range(rng.randrange(0, 4))]
    return rng.choice([1, True, None, {"length": 1, "0": "any"}, {"a": 1}, [1], [None], ["any", 2], "!"])


def platform():
    t = {}
    for k in ["os", "cpu", "libc"]:
        if rng.random() < 0.5:
            t[k] = plat_list() if k == "os" else rng.choice([plat_list(), rng.choice(["x64", "arm64", "!arm64", ["x64", "ia32"], "glibc", "musl", ["!musl"]])])
    return t


NAMES = ["npm", "pnpm", "node", "bun", "yarn", "darwin", "x64", "glibc", ""]


def dep():
    if rng.random() < 0.1:
        return rng.choice([None, "x", 1, [], [{"name": "npm"}]])
    d = {}
    if rng.random() < 0.9:
        d["name"] = rng.choice(NAMES + [1, None])
    if rng.random() < 0.5:
        d["version"] = rng.choice(RANGES + VERS + [1])
    if rng.random() < 0.3:
        d["onFail"] = rng.choice(["ignore", "warn", "error", "download", "explode", 1])
    if rng.random() < 0.05:
        d["extra"] = 1
    return d


def dev():
    if rng.random() < 0.05:
        return rng.choice([None, [], "x", 1])
    return {e: dep() if rng.random() < 0.6 else [dep() for _ in range(rng.randrange(0, 3))]
            for e in rng.sample(["packageManager", "runtime", "cpu", "libc", "os", "bogus", "0"], rng.randrange(0, 4))}


def current():
    if rng.random() < 0.05:
        return rng.choice([None, "x", 1, []])
    return {e: rng.choice([{"name": rng.choice(NAMES), "version": rng.choice(VERS)}, {"name": rng.choice(NAMES)}, {}, "x", None, {"name": 1}])
            for e in ["packageManager", "runtime", "cpu", "libc", "os"] if rng.random() < 0.8}


for _ in range(8):
    o = lambda pool: "_" if rng.random() < 0.15 else hx(rng.choice(pool))
    lines = [f"engine {hx(json.dumps(engines()))} {o(VERS)} {o(VERS)} {rng.randrange(2)}" for _ in range(80)]
    cases.append(("install-engine", "install", case_file(lines), lines))
for _ in range(8):
    lines = [f"platform {hx(json.dumps(platform()))} {int(rng.random() < 0.1)} {hx(rng.choice(['darwin', 'linux', 'win32']))} {hx(rng.choice(['x64', 'arm64', 'ia32']))} "
             f"{'_' if rng.random() < 0.4 else hx(rng.choice(['glibc', 'musl', '']))}" for _ in range(80)]
    cases.append(("install-platform", "install", case_file(lines), lines))
for _ in range(8):
    lines = [f"dev {hx(json.dumps(dev()))} {hx(json.dumps(current()))} {rng.randrange(2)} {rng.randrange(2)}" for _ in range(80)]
    cases.append(("dev-engines", "install", case_file(lines), lines))

# --- npm-pick-manifest ------------------------------------------------------------------------------------

PVERS = ["1.0.0", "1.0.1", "1.1.0", "1.2.0-beta.1", "1.2.0", "2.0.0-rc.1", "2.0.0", "2.1.3", "3.0.0", "0.9.0", "10.0.0", "1.10.0", "2.0.0-alpha", "4.0.0-0"]
PRANGES = ["^1.0.0", "~1.0.0", ">=1.1.0 <2", "*", "1.x", "2", ">=2.0.0-0", "^0.9.0", "<=1.2.0", "1.0.0 - 2.0.0", ">4", "^3 || ^1"]
BINS = [None, "bin/cli.js", "./bin/x", "/abs/path", "..\\..\\evil", ["bin/a.js", "lib/../bin/b.js", "c"],
        {"x": "bin/x.js", "y\\z": "a\\b.js", "": "q", "w": 5, "1": "one.js", "C:x": "d:/e"}, {"../up": "../../x", "a/b": "./", "k": "."}, [], {}, "", 5, ["x", 7]]


def manifest(v):
    m = {"name": rng.choice(["pkg", "@s/pkg", ""]), "version": v}
    if rng.random() < 0.4:
        m["bin"] = rng.choice(BINS)
    if rng.random() < 0.3:
        m["engines"] = rng.choice([{"node": rng.choice([">=18", "<16", "^20 || ^22", ">=99"])}, {"npm": ">=12"}, {}])
    if rng.random() < 0.2:
        m["deprecated"] = rng.choice(["old", "", True])
    return m


def iso(ok=True):
    if not ok:
        return rng.choice(["never", "2020-02-30T00:00:00Z", ""])
    y, mo, d = rng.randrange(2018, 2026), rng.randrange(1, 13), rng.randrange(1, 29)
    f = rng.randrange(5)
    if f == 0:
        return f"{y:04d}-{mo:02d}-{d:02d}"
    if f == 1:
        return f"{y:04d}-{mo:02d}-{d:02d}T{rng.randrange(24):02d}:{rng.randrange(60):02d}:{rng.randrange(60):02d}.{rng.randrange(1000):03d}Z"
    if f == 2:
        return f"{y:04d}-{mo:02d}-{d:02d}T{rng.randrange(24):02d}:{rng.randrange(60):02d}Z"
    if f == 3:
        return f"{y:04d}-{mo:02d}-{d:02d}T{rng.randrange(24):02d}:{rng.randrange(60):02d}:{rng.randrange(60):02d}+0{rng.randrange(10)}:30"
    return f"{y:04d}-{mo:02d}"


def packument():
    p = {"name": "pkg", "versions": {v: manifest(v) for v in rng.sample(PVERS, rng.randrange(0, len(PVERS)))}}
    tags = {t: rng.choice(PVERS) for t in ["latest", "next", "beta"] if rng.random() < 0.7}
    if rng.random() < 0.9:
        p["dist-tags"] = tags
    if rng.random() < 0.7:
        p["time"] = {v: iso(rng.random() < 0.95) for v in PVERS if rng.random() < 0.9}
    if rng.random() < 0.15:
        p["stagedVersions"] = {"versions": {v: manifest(v) for v in rng.sample(PVERS, 2)}}
    if rng.random() < 0.15:
        p["policyRestrictions"] = {"versions": {v: manifest(v) for v in rng.sample(PVERS, 2)}, "message": "no"}
    return p


for _ in range(10):
    lines = []
    for _ in range(60):
        wanted = rng.choice([None, "", "latest", "next", "beta", "nope", "1.0.0", "v2.0.0", "=1.1.0", "9.9.9"] + PRANGES +
                            ["file:../x", "github:a/b", "a/b", "npm:foo@1", "https://x.com/y.tgz", "a b", "é"])
        lines.append(" ".join(["pick", hx(json.dumps(packument())), o_(wanted), o_(rng.choice([None, None, "next", "beta"])), o_(rng.choice([None, None, iso(), iso(), "never"])),
                               hx(rng.choice(["20.0.0", "18.1.0", "22.3.0"])), o_(rng.choice([None, "10.0.0"])), str(rng.randrange(2)),
                               o_(rng.choice([None, None, "^1.0.0", "2.0.0", ">=3", "1.0.1 || 2.1.3"])), str(int(rng.random() < 0.3))]))
    cases.append(("pick-manifest", "pick", case_file(lines), lines))

# --- requirements files ------------------------------------------------------------------------------------

REQS = ["requests>=2.31", "django==4.2.*", "numpy", "pkg[extra]>=1 ; python_version >= \"3.8\"", "./local/pkg", "https://x.org/a.whl", "foo @ https://x/y.tar.gz",
        "a  b", "${PKG}", "${PKG}>=${VER}", "café", "x;y", "name\\", "-", "--", "pkg --hash=sha256:abc --hash=sha256:def", "pkg --hash sha384:zz", "pkg --hash=md5:a",
        "pkg --hash=nocolon", "pkg --global-option=--x --global-option y", "pkg -C k=v -C k=w --config-settings=j=1", "pkg -C novalue", "pkg --config-set a=b",
        "pkg --pre", "pkg -i https://idx"]
OPTS = ["-i https://pypi.example/simple", "--index-url=https://a", "--index https://b", "--pypi-url c", "-ihttps://short", "--extra-index-url https://x", "--ext https://y",
        "--ex https://z", "--no-index", "--no-index=1", "--no", "-f ./links", "--find-links=https://l", "--no-binary :all:", "--no-binary a,B_c,:none:,d.e",
        "--only-binary :all: --no-binary x", "--no-binary=-x", "--only-binary ':none:'", "--prefer-binary", "--require-hashes", "--pre", "--trusted-host h:8080",
        "--use-feature fast-deps", "--use-feature bogus", "--use-feature=truststore", "--bogus", "-x", "-e ./src", "-e", "--editable git+https://g/r.git#egg=r",
        "-i", "--hash=sha256:aa", "-r other.txt", "-r sub/nested.txt", "-c cons.txt", "-r missing.txt", "-r req.txt", "--requirement=../up.txt",
        "-i 'quoted url'", "-i \"unclosed", "-i a\\ b", "-- -i x", "--trusted-host ${HOST}", "--pre extra positional", "-C a=b", "--glob x"]


def req_line():
    r = rng.random()
    if r < 0.35:
        return rng.choice(REQS)
    if r < 0.75:
        return " ".join(rng.choice(OPTS) for _ in range(rng.randrange(1, 3)))
    if r < 0.85:
        return rng.choice(["# comment", "   # indented", "pkg # trailing", "pkg#nospace", "", "   ", "\t"])
    return rng.choice(REQS) + " " + rng.choice(OPTS)


def req_content():
    out = []
    for _ in range(rng.randrange(0, 8)):
        l = req_line()
        if rng.random() < 0.1:
            l = l + " \\"
        if rng.random() < 0.05:
            l = l + "\\\\"
        out.append(l)
    sep = rng.choice(["\n", "\n", "\r\n", "\r", "\x0c", "\u2028"])
    return sep.join(out) + (sep if rng.random() < 0.5 else "")


for i in range(14):
    d = os.path.join(work, f"req{i}", "w")
    files = {"req.txt": req_content()}
    if rng.random() < 0.7:
        files["other.txt"] = req_content()
    if rng.random() < 0.5:
        files["sub/nested.txt"] = req_content() + "\n-r ../other.txt" * (rng.random() < 0.3)
    if rng.random() < 0.4:
        files["cons.txt"] = req_content()
    if rng.random() < 0.1:
        files["../up.txt"] = "upkg"
    lines = [os.path.join(d, "req.txt")]
    for name, text in files.items():
        p = os.path.normpath(os.path.join(d, name))
        os.makedirs(os.path.dirname(p), exist_ok=True)
        open(p, "w", encoding="utf-8").write(text)
        lines.append(f"F\t{p}\t{hx(text)}")
    env = {"PKG": rng.choice(["flask", "", "${VER}"]), "VER": "2.0", "HOST": "h.example"}
    lines += [f"E\t{k}\t{hx(v)}" for k, v in env.items()]
    cases.append(("requirements-file", "reqfile", case_file(lines), [lines[0]]))

# --- entry points --------------------------------------------------------------------------------------------

EVALS = ["black:patched_main", "pytest_cov.plugin", "pkg.mod:obj.attr", "a:b [x, y]", "a : b", "mod [extra1,extra2]", "mod[ex]", "é.mod:ñ", "mod:", ":attr",
         "mod:attr [unclosed", "mod:attr [a] trailing", "mod:attr [a] ]", "mod  ", "a.b.c", "mod:a.b [ x ]  ", "1mod:2", "mod-x:y", "mod:x y", "", "=", "日本:語",
         "mod:attr[x]\u3000", "mod\u2028x"]
ENAMES = ["black", "cov", "my-cli", "x y", "", "=", "é", "a=b"]
GROUPS = ["[console_scripts]", "[pytest11]", "[[weird]]", "[a]]", "[ gui_scripts ]", "[]", "[x", "#[c]", "  [spaced]  "]


def entry_text():
    out = []
    for _ in range(rng.randrange(0, 10)):
        r = rng.random()
        if r < 0.2:
            out.append(rng.choice(GROUPS))
        elif r < 0.8:
            out.append(rng.choice(ENAMES) + rng.choice(["=", " = ", "= ", "=="]) + rng.choice(EVALS))
        elif r < 0.9:
            out.append(rng.choice(["# comment", "", "   ", "\t", "noequals"]))
        else:
            out.append(rng.choice(EVALS))
    return rng.choice(["\n", "\r\n", "\r", "\x0b", "\x1d", "\u2028", "\x85"]).join(out)


for _ in range(8):
    lines = [hx(entry_text()) for _ in range(100)]
    cases.append(("entry-points", "entry", case_file(lines), lines))

# --- run ------------------------------------------------------------------------------------------------------


def oracle(op, paths):
    if op in ("reqfile", "entry"):
        cmd = ["python3.13", os.path.join(LIB, "oracle.py"), op] + paths
    else:
        cmd = ["node", os.path.join(LIB, "oracle.js"), NPM, op] + paths
    r = subprocess.run(cmd, capture_output=True, text=True, env=dict(os.environ, TZ="UTC"), timeout=3600)
    if r.returncode != 0:
        print(r.stderr[-1500:]); sys.exit(1)
    return r.stdout.split("\n")[:-1]


def finish():
    wants = {}
    for op in sorted({c[1] for c in cases}):
        mine = [c for c in cases if c[1] == op]
        for c, w in zip(mine, oracle(op, [c[2] for c in mine])):
            wants[c[2]] = w
    argv = []
    for _, op, path, _ in cases:
        argv += [op, path]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    answers = ran.stdout.split("\n")[:-1]
    if len(answers) != len(cases):
        print(f"{len(answers)} answers for {len(cases)} cases"); sys.exit(1)
    families, differ, inputs = {}, 0, 0
    for (fam, op, path, lines), got in zip(cases, answers):
        want = wants[path]
        ws, gs = want.split(";"), got.split(";")
        inputs += len(lines)
        bad = [k for k in range(max(len(ws), len(gs))) if (ws[k] if k < len(ws) else None) != (gs[k] if k < len(gs) else None)]
        f = families.setdefault(fam, [0, 0])
        if not bad:
            f[0] += 1
            continue
        f[1] += 1
        differ += 1
        if differ <= 8:
            k = bad[0]
            d = lambda x: bytes.fromhex(x).decode(errors="replace") if x else x
            print(f"  {fam} {path} input {k}: {(lines[k] if k < len(lines) else '?')[:200]}")
            print(f"    want  {d(ws[k] if k < len(ws) else '')[:300]!r}")
            print(f"    grasm {d(gs[k] if k < len(gs) else '')[:300]!r}")
    bad_fams = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
    if bad_fams:
        print("disagreeing:", " ".join(bad_fams), f"(SEED={SEED})")
    print("families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
    thin = [k for k, (a, d) in families.items() if a + d < 8]
    if thin or len(families) < int(os.environ.get("FAMILIES", "12")):
        print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
    print(f"cases: {len(cases)} ({inputs} inputs)   agreeing: {len(cases) - differ}   differing: {differ}")
    if differ:
        print(f"files kept in {work}")
        sys.exit(1)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0)


finish()
