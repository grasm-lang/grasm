"""Package graphs over registries, for `deps`, `add` and `publish`
(scripts/clidiff.sh, docs/self-hosting.md §6.21).

    python3 scripts/lib/regcases.py OUTDIR

A registry is a directory, `<root>/<name>/<version>/`, so every registry here
is one under OUTDIR: nothing reaches the network. Writes one tree per case
under OUTDIR and prints the commands to run on them, in order, one a line:

    <case> TAB <working directory, under the case> TAB <env> TAB <arguments>

`<env>` is `-` or `GRASM_REGISTRY=<path under the case>`; the arguments are
split at spaces. `@edit <how> <package>` is no command: it is
`scripts/lib/lockedit.py`, changing that package's lockfile (or a source)
the same way on both sides. The commands of a case run in order on the same tree, so a
`publish` is seen by the `deps` after it. Each case is right or wrong in one
way: the lowest version at or above the highest floor, a conflict, a version
or a name the registry lacks, `^0.x` and `^0.0.x`, version directories that
are not versions, a registry package's own dependencies, a ceiling breached,
a capability that names nothing, a package that cannot be analysed, every
refusal of `publish`, and `add` on manifests of every shape.
"""

import os
import sys

CASES = []
COMMANDS = []

LIB = 'pub fn f() -> i64 {\n    1\n}\n'
MAIN = 'fn main() {\n    print("hi")\n}\n'


def manifest(name, version="0.1.0", caps="", extra="", deps=""):
    # An `app` prints, so it performs `io`; a library performs nothing.
    if caps == "":
        caps = '["io"]' if name == "app" else "[]"
    out = f'[package]\nname = "{name}"\nversion = "{version}"\n'
    if caps is not None:
        out += f"capabilities = {caps}\n"
    out += extra
    if deps:
        out += "\n" + deps
    return out


def dep(name, path=None, version=None):
    out = f"[dependencies.{name}]\n"
    if path is not None:
        out += f'path = "{path}"\n'
    if version is not None:
        out += f'version = "{version}"\n'
    return out


def uses(*names):
    imports = "".join(f"import {n}\n" for n in names)
    body = " + ".join(f"{n}.f()" for n in names) or "0"
    return f'{imports}\nfn main() {{\n    print("{{{body}}}")\n}}\n'


def lib_using(*names):
    imports = "".join(f"import {n}\n" for n in names)
    body = " + ".join(f"{n}.f()" for n in names) or "0"
    return f"{imports}\npub fn f() -> i64 {{\n    {body}\n}}\n"


def reg(files, name, version, caps="[]", deps="", src=LIB, mf_version=None, mf_name=None):
    base = f".grasm/registry/{name}/{version}"
    files[f"{base}/grasm.toml"] = manifest(mf_name or name, mf_version or version, caps, deps=deps)
    files[f"{base}/src/main.gr"] = src


def case(name, files, commands, modes=None):
    CASES.append((name, files, modes or {}))
    for cwd, env, args in commands:
        COMMANDS.append((name, cwd, env, args))


def deps_case(name, files):
    case(name, files, [(".", "-", "deps app"), (".", "-", "deps app --locked"), ("app", "-", "deps")])


# --- resolution against a registry ------------------------------------------

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.1.0")), "app/src/main.gr": uses("far")}
for v in ["1.0.0", "1.1.5", "1.2.0", "2.0.0"]:
    reg(f, "far", v)
deps_case("lowest-at-floor", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0") + dep("mid", version="^1.0.0")),
     "app/src/main.gr": uses("far", "mid")}
for v in ["1.0.0", "1.2.0", "1.3.0"]:
    reg(f, "far", v)
reg(f, "mid", "1.0.0", deps=dep("far", version="^1.2.0"), src=lib_using("far"))
deps_case("highest-floor", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="=1.0.0") + dep("mid", version="^1.0.0")),
     "app/src/main.gr": uses("far", "mid")}
for v in ["1.0.0", "1.2.0"]:
    reg(f, "far", v)
reg(f, "mid", "1.0.0", deps=dep("far", version="^1.2.0"), src=lib_using("far"))
deps_case("conflict", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^3.0.0")), "app/src/main.gr": uses("far")}
for v in ["1.0.0", "2.5.1"]:
    reg(f, "far", v)
deps_case("version-missing", f)

f = {"app/grasm.toml": manifest("app", deps=dep("ghost", version="^1.0.0")), "app/src/main.gr": uses("ghost"),
     ".grasm/registry/far/1.0.0/grasm.toml": manifest("far", "1.0.0"), ".grasm/registry/far/1.0.0/src/main.gr": LIB}
deps_case("name-missing", f)

f = {"app/grasm.toml": manifest("app", deps=dep("z", version="^0.2.0") + dep("q", version="^0.0.3")),
     "app/src/main.gr": uses("z", "q")}
for v in ["0.2.0", "0.2.5", "0.3.0"]:
    reg(f, "z", v)
for v in ["0.0.3", "0.0.4"]:
    reg(f, "q", v)
deps_case("zero-ranges", f)

f = {"app/grasm.toml": manifest("app", deps=dep("q", version="^0.0.3") + dep("mid", version="^1.0.0")),
     "app/src/main.gr": uses("q", "mid")}
for v in ["0.0.3", "0.0.4"]:
    reg(f, "q", v)
reg(f, "mid", "1.0.0", deps=dep("q", version="^0.0.4"), src=lib_using("q"))
deps_case("zero-zero-conflict", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0")), "app/src/main.gr": uses("far"),
     ".grasm/registry/far/README.md": "versions live beside this file\n",
     ".grasm/registry/far/latest/grasm.toml": manifest("far", "9.9.9"),
     ".grasm/registry/far/1.0/grasm.toml": manifest("far", "1.0.0"),
     ".grasm/registry/far/1.5.0/src/main.gr": LIB}
reg(f, "far", "1.2.0")
deps_case("not-versions", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0")), "app/src/main.gr": uses("far")}
reg(f, "far", "1.0.0", mf_version="1.4.0")
deps_case("version-dir-mismatch", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0")), "app/src/main.gr": uses("far")}
reg(f, "far", "1.0.0", mf_name="other")
deps_case("registry-wrong-name", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0")), "app/src/main.gr": uses("far"),
     ".grasm/registry/far/1.0.0/grasm.toml": manifest("far", "1.0.0", deps=dep("util", path="../../util")),
     ".grasm/registry/far/1.0.0/src/main.gr": lib_using("util"),
     ".grasm/registry/util/grasm.toml": manifest("util", "0.1.0"),
     ".grasm/registry/util/src/main.gr": LIB}
deps_case("registry-path-dependency", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0")), "app/src/main.gr": uses("far")}
reg(f, "far", "1.0.0", src='pub fn f() -> i64 {\n    print("touched")\n    1\n}\n')
deps_case("registry-ceiling", f)

f = {"app/grasm.toml": manifest("app", caps='["io", "nett", "Clok", "Clock", "zzzzzz"]'),
     "app/src/main.gr": 'effect Clock {\n    fn now() -> i64\n}\n\nfn main() {\n    print("hi")\n}\n'}
deps_case("capability-names", f)

f = {"app/grasm.toml": manifest("app", caps=None, deps=dep("lib", path="../lib")),
     "app/src/main.gr": uses("lib"),
     "lib/grasm.toml": manifest("lib", "1.0.0"),
     "lib/src/main.gr": LIB,
     "lib/src/broken.gr": "pub fn g( -> i64 {\n",
     "lib/tests/also.gr": "pub fn h() -> i64 {\n    nope\n}\n"}
deps_case("cannot-analyze", f)

f = {"app/grasm.toml": manifest("app", deps=dep("lib", path="../lib")),
     "app/src/main.gr": uses("lib"),
     "lib/grasm.toml": manifest("lib", "1.0.0"),
     "lib/src/main.gr": "import helper\n\npub fn f() -> i64 {\n    helper.g()\n}\n",
     "lib/src/helper.gr": "pub fn g() -> i64 {\n    \"not a number\"\n}\n"}
deps_case("reached-through", f)

f = {"app/grasm.toml": manifest("app", caps='["io"]', deps=dep("lib", path="../lib")),
     "app/src/main.gr": uses("lib"),
     "lib/grasm.toml": manifest("lib", "1.0.0"),
     "lib/src/main.gr": LIB,
     "lib/src/extra.gr": 'pub fn leak() {\n    print("reach")\n}\n',
     "lib/README.GR": 'pub fn shout() {\n    print("upper")\n}\n'}
deps_case("sibling-module", f)

f = {"app/grasm.toml": manifest("app", caps="[]", deps=dep("lib", path="../lib")),
     "app/src/main.gr": uses("lib")[:-2] + '    print("x")\n}\n',
     "lib/grasm.toml": manifest("lib", "1.0.0", caps=None),
     "lib/src/main.gr": LIB}
deps_case("root-over-ceiling", f)

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0")), "app/src/main.gr": uses("far")}
deps_case("no-registry", f)

# GRASM_REGISTRY, and a registry found by walking up from a nested package.
f = {"work/app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0")), "work/app/src/main.gr": uses("far"),
     "elsewhere/far/1.0.0/grasm.toml": manifest("far", "1.0.0"), "elsewhere/far/1.0.0/src/main.gr": LIB,
     "elsewhere/far/1.1.0/grasm.toml": manifest("far", "1.1.0"), "elsewhere/far/1.1.0/src/main.gr": LIB}
case("env-registry", f, [(".", "GRASM_REGISTRY=elsewhere", "deps work/app"),
                         ("work/app", "GRASM_REGISTRY=../../elsewhere", "deps"),
                         (".", "-", "deps work/app")])

f = {"top/.grasm/registry/far/1.0.0/grasm.toml": manifest("far", "1.0.0"),
     "top/.grasm/registry/far/1.0.0/src/main.gr": LIB,
     "top/a/b/app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0")),
     "top/a/b/app/src/main.gr": uses("far")}
case("discovered-registry", f, [(".", "-", "deps top/a/b/app"), ("top/a/b/app", "-", "deps"),
                                ("top/a", "-", "deps b/app --locked")])

# --- the lockfile, edited every way `diff_lock` names -----------------------

f = {"app/grasm.toml": manifest("app", deps=dep("far", version="^1.0.0") + dep("lib", path="../lib")),
     "app/src/main.gr": uses("far", "lib"),
     "lib/grasm.toml": manifest("lib", "1.0.0", caps='["io", "async"]'),
     "lib/src/main.gr": LIB}
reg(f, "far", "1.0.0")
reg(f, "far", "1.1.0")
case("lockfile", f, [(".", "-", "deps app --locked"), (".", "-", "deps app"), (".", "-", "deps app --locked")])
EDITS = ["checksum", "version", "caps-none", "caps-empty", "caps-order", "removed", "added", "duplicate",
         "key-twice", "unreadable", "missing-name", "not-utf8", "reordered", "source"]
for e in EDITS:
    COMMANDS.append(("lockfile", ".", "-", f"@edit {e} app"))
    COMMANDS.append(("lockfile", ".", "-", "deps app --locked"))
    COMMANDS.append(("lockfile", ".", "-", "deps app"))

# --- publish ----------------------------------------------------------------

f = {"pkg/grasm.toml": manifest("pkg", "+01.002.3", caps="[]"),
     "pkg/src/main.gr": LIB,
     "pkg/src/util/more.gr": LIB,
     "pkg/tests/t.gr": LIB,
     "pkg/LOUD.GR": LIB,
     "pkg/README.md": "not published\n",
     "pkg/grasm.lock": "# not published\n",
     "pkg/target/x.gr": LIB,
     "pkg/.hidden/y.gr": LIB,
     "pkg/.gr": "not a source\n",
     "nocaps/grasm.toml": manifest("nocaps", caps=None), "nocaps/src/main.gr": LIB,
     "over/grasm.toml": manifest("over", caps="[]"),
     "over/src/main.gr": 'pub fn f() {\n    print("io")\n}\n',
     "pathdep/grasm.toml": manifest("pathdep", deps=dep("pkg", path="../pkg")),
     "pathdep/src/main.gr": lib_using("pkg"),
     "noentry/grasm.toml": manifest("noentry", extra='entry = "src/gone.gr"\n'),
     "user/grasm.toml": manifest("user", caps='["io"]', deps=dep("pkg", version="^1.2.0")),
     "user/src/main.gr": uses("pkg"),
     "badcheck/grasm.toml": manifest("badcheck"),
     "badcheck/src/main.gr": "pub fn f() -> i64 {\n    \"no\"\n}\n"}
modes = {"pkg/src/util/more.gr": 0o755, "pkg/tests/t.gr": 0o600}
case("publish", f, [
    (".", "-", "publish pkg --registry reg"),
    (".", "-", "publish pkg --registry reg"),
    ("pkg", "-", "publish --registry ../reg2"),
    (".", "GRASM_REGISTRY=reg3", "publish pkg"),
    (".", "-", "publish pkg"),
    (".", "-", "publish nocaps --registry reg"),
    (".", "-", "publish over --registry reg"),
    (".", "-", "publish pathdep --registry reg"),
    (".", "-", "publish noentry --registry reg"),
    (".", "-", "publish badcheck --registry reg"),
    (".", "-", "publish missing --registry reg"),
    (".", "GRASM_REGISTRY=reg", "deps user"),
], modes)

f = {"top/.grasm/registry/README": "the registry\n",
     "top/lib/grasm.toml": manifest("lib", "2.0.0"), "top/lib/src/main.gr": LIB,
     "top/app/grasm.toml": manifest("app", deps=dep("lib", version="^2.0.0")), "top/app/src/main.gr": uses("lib")}
case("publish-discovered", f, [("top/lib", "-", "publish"), (".", "-", "publish top/lib"),
                               (".", "-", "deps top/app")])

# --- add --------------------------------------------------------------------

f = {"app/grasm.toml": manifest("app", deps=dep("lib", path="../lib") + dep("far", version="^1.0.0")),
     "app/src/main.gr": MAIN,
     "nonl/grasm.toml": manifest("nonl").rstrip("\n"),
     "crlf/grasm.toml": manifest("crlf").replace("\n", "\r\n"),
     "broken/grasm.toml": "[package]\nname = 1.5\n",
     "bad/grasm.toml": manifest("bad", extra="entrypoint = \"x\"\n"),
     "empty/.keep": "",
     "here/grasm.toml": manifest("here")}
cmds = []
for args in ["add json 1.2.3 app", "add json ^1.0.0 app", "add lib 1.0.0 app", "add far 2.0.0 app",
             "add net_x =2.0.0 app", "add plus +01.2.3 app", "add v 1.2 app",
             "add v x app", "add v =2 app", "add my-dep 1.0.0 app", "add match 1.0.0 app",
             "add ../evil 1.0.0 app", "add ok 1.0.0 nonl", "add ok 1.0.0 crlf", "add ok 1.0.0 broken",
             "add ok 1.0.0 bad", "add ok 1.0.0 empty", "add ok 1.0.0 nowhere", "add ok 1.0.0 app --x",
             "add ok 1.0.0"]:
    cmds.append((".", "-", args))
cmds.append(("here", "-", "add ok ^0.0.1"))
cmds.append(("here", "-", "add ok2 0.2.0 ."))
case("add", f, cmds)
NOT_UTF8 = {"add": ["notutf8/grasm.toml"]}
COMMANDS.append(("add", ".", "-", "add ok 1.0.0 notutf8"))


def write(out):
    for name, files, modes in CASES:
        for rel, text in files.items():
            path = os.path.join(out, name, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8", newline="") as fh:
                fh.write(text)
        for rel, mode in modes.items():
            os.chmod(os.path.join(out, name, rel), mode)
        for rel in NOT_UTF8.get(name, []):
            path = os.path.join(out, name, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as fh:
                fh.write(b'[package]\nname = "\xff"\nversion = "0.1.0"\n')


def main():
    out = sys.argv[1]
    write(out)
    for name, cwd, env, args in COMMANDS:
        print(f"{name}\t{cwd}\t{env}\t{args}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
