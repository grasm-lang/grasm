"""Package-ecosystem files against the tools that own them.

- `sri` against npm's ssri and `npmname` against validate-npm-package-name,
  both loaded from the npm that ships with Node (scripts/lib/node/b18.js).
- `gosum`'s h1 hashes against golang.org/x/mod/sumdb/dirhash, over file
  sets and over zips Python writes (scripts/lib/gomods); its go.sum
  reading against the field rule of cmd/go.
- `gemfilelock` against Bundler's LockfileParser (scripts/lib/ruby).
- `pkgmetadata` against importlib.metadata's own reading of a METADATA file.
"""
import base64
import email
import hashlib
import io
import json
import os
import random
import subprocess
import sys
import tempfile
import warnings
import zipfile
from importlib.metadata._adapters import Message

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts/lib/gomods"))
import oracle as gomods  # noqa: E402

rng = random.Random(20261018)
# Zips with a repeated name are written on purpose: Go hashes the last.
warnings.filterwarnings("ignore", "Duplicate name")
work = tempfile.mkdtemp(prefix="b18diff")
harness = os.path.join(work, "b18test")
built = subprocess.run([GRASM, "build", "packages/b18test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)
go_oracle = gomods.build(os.path.join(work, "gomods"))
US, RS = "\x1f", "\x1e"
cases = []           # op, a, b, want (filled later for oracle families)
node_reqs, ruby_reqs, go_reqs = [], [], []


def hx(s):
    return (s.encode() if isinstance(s, str) else s).hex()


def via(store, op, a, b, request):
    cases.append([op, a, b, None, store, len(store)])
    store.append(request)


def local(op, a, b, want):
    cases.append([op, a, b, want, None, None])


# --- sri -----------------------------------------------------------------------------------
def b64(n):
    return base64.b64encode(bytes(rng.getrandbits(8) for _ in range(n))).decode()


def sri_word():
    algo = rng.choice(["sha512", "sha384", "sha256", "sha1", "md5", "sha224", "123", "07", "abc9", "Sha512", "sha3"])
    digest = rng.choice([b64(64), b64(48), b64(32), b64(20), "abc", "a+/=", "x?y", "dGVzdA==", "", b64(70)])
    opts = rng.choice(["", "?foo", "?foo?bar", "??", "?a b", "?x\x7f", "?ok?"])
    sep = rng.choice(["-", "-", "-", "_", ""])
    return algo + sep + digest + opts


for _ in range(300):
    text = rng.choice([" ", "\t", "  \n "]).join(sri_word() for _ in range(rng.randrange(0, 4)))
    if rng.random() < 0.1:
        text = " " + text + " "
    mode = rng.choice(["strict", "lenient"])
    via(node_reqs, "sri.parse", hx(text), mode, {"op": "sri.parse", "a": hx(text), "b": mode})
for _ in range(150):
    data = bytes(rng.getrandbits(8) for _ in range(rng.randrange(0, 50)))
    algos = rng.sample(["sha512", "sha384", "sha256", "sha1", "md5"], rng.randrange(1, 4))
    good = " ".join(f"{a}-{base64.b64encode(hashlib.new(a, data).digest()).decode()}" for a in algos)
    bad = " ".join(f"{a}-{b64(32)}" for a in algos)
    integrity = rng.choice([good, bad, good + " " + bad, "sha512-" + b64(64) + " " + good])
    via(node_reqs, "sri.check", data.hex(), hx(integrity), {"op": "sri.check", "a": data.hex(), "b": hx(integrity)})
    other = rng.choice([good, bad, integrity, "sha1-" + b64(20)])
    via(node_reqs, "sri.match", hx(integrity), hx(other), {"op": "sri.match", "a": hx(integrity), "b": hx(other)})
    b = ",".join(algos) + US + ",".join(rng.sample(["foo", "bar", "baz"], rng.randrange(0, 3)))
    via(node_reqs, "sri.from", data.hex(), b, {"op": "sri.from", "a": data.hex(), "b": b})


# --- npm names -----------------------------------------------------------------------------
PIECES = ["lodash", "@scope/pkg", "Http", "http", "fs", "node_modules", "favicon.ico", ".hidden", "_under", " sp ",
          "a b", "@scope/.x", "@/x", "a/b", "@a/b/c", "x~y", "ok-name", "café", "UPPER", "a" * 215, "@sc ope/pkg",
          "node:fs", "crypto", "Crypto", "timers/promises", "@types/node", "x!", "(x)", "%", "　x", "x "]
for _ in range(250):
    name = rng.choice(PIECES) if rng.random() < 0.6 else "".join(rng.choice(PIECES) for _ in range(rng.randrange(1, 3)))
    via(node_reqs, "npm.name", hx(name), "-", {"op": "npm.name", "a": hx(name)})


# --- go.sum and h1 ---------------------------------------------------------------------------
for _ in range(150):
    files = []
    for _ in range(rng.randrange(0, 6)):
        name = rng.choice(["go.mod", "main.go", "a/b.go", "README", "z", "LICENSE", "é.txt", "x/y/z.go"])
        files.append((name, bytes(rng.getrandbits(8) for _ in range(rng.randrange(0, 40)))))
    seen, uniq = set(), []
    for n, d in files:
        if n not in seen:
            seen.add(n)
            uniq.append((n, d))
    if rng.random() < 0.05:
        uniq.append(("bad\nname", b"x"))
    a = ",".join(f"{hx(n)}:{d.hex()}" for n, d in uniq)
    hexes = [x for n, d in uniq for x in (hx(n), d.hex())]
    via(go_reqs, "gosum.h1", a, "-", {"op": "h1", "hexes": hexes})
for _ in range(120):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", rng.choice([zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED])) as z:
        for _ in range(rng.randrange(1, 5)):
            name = f"example.com/m@v1.0.0/{rng.choice(['go.mod', 'a.go', 'b/c.go', 'README'])}"
            z.writestr(name, bytes(rng.getrandbits(8) for _ in range(rng.randrange(0, 60))))
    data = buf.getvalue()
    via(go_reqs, "gosum.zip", data.hex(), "-", {"op": "h1zip", "hex": data.hex()})
for _ in range(120):
    lines = []
    for _ in range(rng.randrange(0, 6)):
        mod = rng.choice(["golang.org/x/net", "github.com/a/b", "example.com/m"])
        ver = rng.choice(["v1.2.3", "v0.0.0-20200101000000-abcdefabcdef", "v2.0.0+incompatible"])
        if rng.random() < 0.5:
            ver += "/go.mod"
        line = f"{mod} {ver} h1:{b64(32)}"
        if rng.random() < 0.08:
            line = rng.choice([line + " extra", mod + " " + ver])
        lines.append(rng.choice(["", "  "]) + line.replace(" ", rng.choice([" ", "\t", "  "])))
        if rng.random() < 0.2:
            lines.append("")
    text = "\n".join(lines) + rng.choice(["", "\n"])
    fields = [l.split() for l in text.split("\n")]
    if any(len(f) not in (0, 3) for f in fields):
        want = "ERR"
    else:
        want = RS.join(f"{f[0]}{US}{f[1][:-7] if f[1].endswith('/go.mod') else f[1]}{US}"
                       f"{'true' if f[1].endswith('/go.mod') else 'false'}{US}{f[2]}" for f in fields if f)
    local("gosum.parse", hx(text), "-", want)


# --- Gemfile.lock ---------------------------------------------------------------------------
GEMS = ["rake", "rails", "nokogiri", "json", "rack", "puma", "mini_portile2", "racc", "foo-bar", "zeitwerk"]
VERSIONS = ["13.0.1", "1.0", "2.5.0.rc1", "1.0-beta", " 3.2 ", "0.0.1", "10.0", "1.10.0", "1.9.9"]
PLATFORMS = ["", "-x86_64-linux", "-java", "-i686-linux", "-x86_64-darwin19", "-arm64-darwin-21", "-x64-mingw32",
             "-universal-darwin", "-arm-linux-gnueabihf", "-x86-mswin32", "-sparc-solaris2.8"]
REQS = ["", " (~> 1.0)", " (>= 0)", " (= 2.0)", " (>= 1.2, < 2)", " (< 2, >= 1.2)", " (~> 1.0, >= 1.0.2)", " (> 1)",
        " (!= 1.5)", " (1.2)"]


def gem_lock():
    out = []
    names = rng.sample(GEMS, rng.randrange(1, 6))
    unused = list(names)
    for kind in rng.sample(["GEM", "GIT", "PATH"], rng.randrange(1, 4)):
        out.append(kind)
        if kind == "GEM":
            out.append("  remote: " + rng.choice(["https://rubygems.org/", "https://rubygems.org", "https://gems.example.com/"]))
            if rng.random() < 0.3:
                out.append("  remote: https://other.example/")
        elif kind == "GIT":
            out.append("  remote: https://github.com/x/" + rng.choice(GEMS) + ".git")
            out.append("  revision: " + "".join(rng.choice("0123456789abcdef") for _ in range(40)))
            if rng.random() < 0.5:
                out.append("  branch: main")
        else:
            out.append("  remote: " + rng.choice(["vendor/x", ".", "../lib"]))
        out.append("  specs:")
        # Each gem in one source only: Ruby's sort_by is unstable on the
        # ties a gem repeated across sources would make.
        mine = rng.sample(unused, rng.randrange(0, len(unused) + 1))
        unused = [n for n in unused if n not in mine]
        for n in [x for x in mine for _ in range(rng.randrange(1, 3))]:
            out.append(f"    {n} ({rng.choice(VERSIONS)}{rng.choice(PLATFORMS)})")
            for d in rng.sample(GEMS, rng.randrange(0, 3)):
                out.append(f"      {d}{rng.choice(REQS)}")
        out.append("")
    out.append("PLATFORMS")
    for p in rng.sample(["ruby", "x86_64-linux", "java", "i686-linux", "x86_64-darwin19", "arm64-darwin-21"], rng.randrange(1, 4)):
        out.append("  " + p)
    out.append("")
    out.append("DEPENDENCIES")
    for n in rng.sample(GEMS, rng.randrange(0, 5)):
        out.append(f"  {n}{rng.choice(REQS)}{rng.choice(['', '', '!'])}")
    out.append("")
    if rng.random() < 0.5:
        out += ["RUBY VERSION", "   ruby 2.6.10p210", ""]
    if rng.random() < 0.7:
        out += ["BUNDLED WITH", "   " + rng.choice(["1.17.2", "1.17.1", "1.16.6"]), ""]
    text = "\n".join(out)
    if rng.random() < 0.04:
        text = text.replace("(13.0.1", "(1..2", 1).replace("(1.0", "(1..2", 1)
    if rng.random() < 0.03:
        text += "<<<<<<< HEAD\n"
    return text


for _ in range(220):
    text = gem_lock()
    via(ruby_reqs, "gem.lock", hx(text), "-", hx(text))


# --- METADATA -------------------------------------------------------------------------------
def metadata():
    out = ["Metadata-Version: 2.1", f"Name: {rng.choice(['requests', 'my-pkg', 'Foo_Bar'])}", "Version: 1.0.0"]
    fields = ["Summary: A thing", "Home-page: https://example.com", "Author: A. Person", "License: MIT",
              "Classifier: Programming Language :: Python :: 3", "Classifier: License :: OSI Approved :: MIT License",
              "Requires-Dist: urllib3 (<3,>=1.21.1)", "Requires-Dist: idna; extra == \"socks\"",
              "Project-URL: Source, https://github.com/x/y", "Keywords: http  web\tclient", "Keywords: second",
              "Provides-Extra: socks", "requires-dist: lower-case-key", "Platform: any",
              "Description: first line\n        |  indented\n        |\n        last", "Requires-Python: >=3.8",
              "Dynamic: license", "X-Custom: value", "License: multi\n line\n\tvalue", "Summary:   spaced",
              "Description-Content-Type: text/markdown"]
    out += rng.sample(fields, rng.randrange(0, 10))
    text = "\n".join(out)
    if rng.random() < 0.6:
        text += "\n\n" + rng.choice(["# Title\n\nSome body text.\n", "body", "line\n\n\nmore\n", "   indented body\n"])
    elif rng.random() < 0.5:
        text += "\n"
    if rng.random() < 0.05:
        text = " leading continuation\n" + text
    if rng.random() < 0.05:
        text = text.replace("Version: 1.0.0", "Not a header line\nVersion: 1.0.0")
    return text


def meta_want(text):
    m = Message(email.message_from_string(text))
    out = []
    for k, v in m.json.items():
        if isinstance(v, list):
            out.append(k + US + "*" + US + (US + US).join(v))
        else:
            out.append(k + US + "1" + US + v)
    return RS.join(out).replace("\n", "\x1d")


for _ in range(220):
    text = metadata()
    local("meta.json", hx(text), "-", meta_want(text))


# --- run the oracles -----------------------------------------------------------------------
node_ans = json.loads(subprocess.run(["node", "scripts/lib/node/b18.js"], input=json.dumps(node_reqs), capture_output=True,
                                     text=True, cwd=ROOT, check=True).stdout)
ruby_run = subprocess.run(["ruby", os.path.join(ROOT, "scripts/lib/ruby/lockfile.rb")], input=json.dumps(ruby_reqs),
                          capture_output=True, text=True)
if ruby_run.returncode != 0:
    print(ruby_run.stderr[-1500:]); sys.exit(1)
ruby_ans = json.loads(ruby_run.stdout)
go_ans = json.loads(subprocess.run([go_oracle], input=json.dumps(go_reqs), capture_output=True, text=True, check=True).stdout)
for c in cases:
    if c[4] is node_reqs:
        c[3] = node_ans[c[5]]
    elif c[4] is ruby_reqs:
        c[3] = ruby_ans[c[5]]
    elif c[4] is go_reqs:
        c[3] = go_ans[c[5]]


def run(picked):
    argv = []
    for c in picked:
        argv += [c[0], c[1], c[2]]
    lines, start = [], 0
    while start < len(argv):
        chunk = argv[start:start + 600]
        start += 600
        try:
            out = subprocess.run([harness] + chunk, capture_output=True, text=True, timeout=300)
        except subprocess.TimeoutExpired:
            print("the harness did not finish in 300 s"); sys.exit(1)
        if out.returncode != 0:
            print(out.stderr[-2000:]); sys.exit(1)
        lines += out.stdout.split("\n")[:-1]
    return lines


got = run(cases)
families = {}
agree = differ = 0
for i, c in enumerate(cases):
    mine = got[i] if i < len(got) else "<missing>"
    fam = families.setdefault(c[0].split(".")[0], [0, 0])
    if mine == c[3]:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        only = os.environ.get("ONLY", "")
        if c[0].startswith(only) and (differ <= 8 or only):
            a = bytes.fromhex(c[1]).decode(errors="replace") if c[0] not in ("gosum.h1", "gosum.zip", "sri.check", "sri.from") else c[1]
            print(f"  {c[0]} {a[:300]!r} {c[2][:80]!r}")
            print(f"    grasm {mine[:400]!r}")
            print(f"    want  {c[3][:400]!r}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 5:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
