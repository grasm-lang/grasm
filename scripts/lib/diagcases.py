"""Package graphs that reach the manifest's, the resolver's, the loader's and
the capability ceilings' diagnostics.

    python3 scripts/lib/diagcases.py OUTDIR

Nothing in the repository has a broken manifest -- `tomldiff` reads every one
alike -- so `diagdiff` writes these instead: one tree per case under OUTDIR,
each a small program in a package whose manifest, dependencies, imports or
effects are wrong in one way (and a few that are right in an unusual way: a
dependency reached through a symbolic link and by its real path, one named
relatively and absolutely, a manifest with Windows line ends). It prints the
entry file of each, one a line, for `grasm check` and `front check` to
compare.
"""

import os
import sys

MAIN = 'fn main() {\n    print("hi")\n}\n'
LIB = 'pub fn f() -> i64 {\n    1\n}\n'


def manifest(name="app", version="0.1.0", extra="", deps=""):
    return f'[package]\nname = "{name}"\nversion = "{version}"\n{extra}\n{deps}'


def dep(name, path=None, version=None):
    out = f"[dependencies.{name}]\n"
    if path is not None:
        out += f'path = "{path}"\n'
    if version is not None:
        out += f'version = "{version}"\n'
    return out


# Each case: name -> {relative path: text}; the entry is app/src/main.gr.
CASES = {}


def case(name, files, entry="app/src/main.gr"):
    CASES[name] = (files, entry)


def bad_manifest(name, text):
    case(name, {"app/grasm.toml": text, "app/src/main.gr": MAIN})


# The TOML reader's own errors.
bad_manifest("toml-unclosed-table", "[package\nname = \"app\"\n")
bad_manifest("toml-unclosed-array-table", "[[bin]\n")
bad_manifest("toml-empty-table", "[ ]\n")
bad_manifest("toml-empty-array-table", "[[ ]]\n")
bad_manifest("toml-table-twice", manifest() + "[package]\n")
bad_manifest("toml-no-equals", "[package]\nname\n")
bad_manifest("toml-empty-key", '[package]\n= "x"\n')
bad_manifest("toml-unterminated-string", '[package]\nname = "app\n')
bad_manifest("toml-unterminated-array", '[package]\nname = "app"\ncapabilities = ["io"\n')
bad_manifest("toml-float", "[package]\nname = 1.5\n")
bad_manifest("toml-key-twice", '[package]\nname = "a"\nname = "b"\n')
bad_manifest("toml-bad-item", '[package]\ncapabilities = ["io", x]\n')
# Manifest::parse, in its order.
bad_manifest("no-package", '[dependencies.x]\npath = "../x"\n')
bad_manifest("no-name", '[package]\nversion = "0.1.0"\n')
bad_manifest("name-not-string", '[package]\nname = 3\nversion = "0.1.0"\n')
bad_manifest("empty-name", '[package]\nname = ""\nversion = "0.1.0"\n')
bad_manifest("no-version", '[package]\nname = "app"\n')
bad_manifest("bad-version", manifest(version="1.0"))
bad_manifest("bad-version-word", manifest(version="1.0.x"))
bad_manifest("caps-not-strings", manifest(extra="capabilities = [1]"))
bad_manifest("caps-not-array", manifest(extra='capabilities = "io"'))
bad_manifest("links-not-array", manifest(extra='links = "m"'))
bad_manifest("links-bad-name", manifest(extra='links = ["-lm"]'))
bad_manifest("entry-absolute", manifest(extra='entry = "/tmp/x.gr"'))
bad_manifest("entry-parent", manifest(extra='entry = "../x.gr"'))
bad_manifest("entry-not-gr", manifest(extra='entry = "src/main.txt"'))
bad_manifest("entry-dot-gr", manifest(extra='entry = "src/.gr"'))
bad_manifest("runtime-not-bool", manifest(extra='runtime = "yes"'))
bad_manifest("dep-version-not-string", manifest(deps="[dependencies.x]\nversion = 1\n"))
bad_manifest("dep-bad-version", manifest(deps=dep("x", version="^1.x")))
bad_manifest("dep-bad-exact", manifest(deps=dep("x", version="=2")))
bad_manifest("dep-neither", manifest(deps="[dependencies.x]\n"))
bad_manifest("dep-empty-name", manifest(deps='[dependencies.]\npath = "../x"\n'))
bad_manifest("package-unknown-key", manifest(extra='entrypoint = "src/main.gr"\nauthors = ["a"]'))
bad_manifest("root-key", 'edition = "2026"\n' + manifest())
bad_manifest("unknown-table", manifest() + "[capabilities]\nio = true\n")
bad_manifest("dep-unknown-key", manifest(deps=dep("x", path="../x") + 'features = ["a"]\n'))
bad_manifest("name-not-identifier", manifest(name="my-app"))
bad_manifest("name-keyword", manifest(name="match"))
# Accepted, written unusually.
case("crlf-manifest", {"app/grasm.toml": manifest().replace("\n", "\r\n"), "app/src/main.gr": MAIN})
case("version-normalized", {"app/grasm.toml": manifest(version="+01.002.3", deps=dep("lib", path="../lib", version="^1.02.0")),
                            "app/src/main.gr": "import lib\n" + MAIN,
                            "lib/grasm.toml": manifest(name="lib", version="1.2.0"), "lib/src/main.gr": LIB})
case("entry-upper-gr", {"app/grasm.toml": manifest(extra='entry = "src/Main.GR"'), "app/src/Main.GR": MAIN},
     entry="app/src/Main.GR")

# The resolver.
case("dep-missing", {"app/grasm.toml": manifest(deps=dep("lib", path="../nowhere")), "app/src/main.gr": MAIN})
case("dep-wrong-name", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")), "app/src/main.gr": MAIN,
                        "lib/grasm.toml": manifest(name="other"), "lib/src/main.gr": LIB})
case("dep-too-old", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib", version="^2.0.0")), "app/src/main.gr": MAIN,
                     "lib/grasm.toml": manifest(name="lib", version="1.4.0"), "lib/src/main.gr": LIB})
case("dep-not-exact", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib", version="=1.4.1")), "app/src/main.gr": MAIN,
                       "lib/grasm.toml": manifest(name="lib", version="1.4.0"), "lib/src/main.gr": LIB})
case("dep-cycle", {"app/grasm.toml": manifest(deps=dep("a", path="../a")), "app/src/main.gr": MAIN,
                   "a/grasm.toml": manifest(name="a", deps=dep("b", path="../b")), "a/src/main.gr": LIB,
                   "b/grasm.toml": manifest(name="b", deps=dep("a", path="../a")), "b/src/main.gr": LIB})
case("dep-bad-manifest", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")), "app/src/main.gr": MAIN,
                          "lib/grasm.toml": "[package]\nname = \"lib\"\n", "lib/src/main.gr": LIB})
case("two-directories", {"app/grasm.toml": manifest(deps=dep("b", path="../b") + dep("c", path="../c")),
                         "app/src/main.gr": MAIN,
                         "b/grasm.toml": manifest(name="b", deps=dep("lib", path="../lib1")), "b/src/main.gr": LIB,
                         "c/grasm.toml": manifest(name="c", deps=dep("lib", path="../lib2")), "c/src/main.gr": LIB,
                         "lib1/grasm.toml": manifest(name="lib", version="1.0.0"), "lib1/src/main.gr": LIB,
                         "lib2/grasm.toml": manifest(name="lib", version="1.1.0"), "lib2/src/main.gr": LIB})
case("no-registry", {"app/grasm.toml": manifest(deps=dep("faraway", version="^1.0.0")), "app/src/main.gr": MAIN})
case("registry-no-package", {"app/grasm.toml": manifest(deps=dep("faraway", version="^1.0.0")), "app/src/main.gr": MAIN,
                             ".grasm/registry/README": "a registry\n"})
case("registry-too-old", {"app/grasm.toml": manifest(deps=dep("far", version="^2.0.0")), "app/src/main.gr": MAIN,
                          ".grasm/registry/far/1.0.0/grasm.toml": manifest(name="far", version="1.0.0"),
                          ".grasm/registry/far/1.0.0/src/main.gr": LIB,
                          ".grasm/registry/far/1.3.0/grasm.toml": manifest(name="far", version="1.3.0"),
                          ".grasm/registry/far/1.3.0/src/main.gr": LIB})
case("registry-conflict", {"app/grasm.toml": manifest(deps=dep("far", version="^1.0.0") + dep("mid", path="../mid")),
                           "app/src/main.gr": MAIN,
                           "mid/grasm.toml": manifest(name="mid", deps=dep("far", version="^2.0.0")), "mid/src/main.gr": LIB,
                           ".grasm/registry/far/1.0.0/grasm.toml": manifest(name="far", version="1.0.0"),
                           ".grasm/registry/far/1.0.0/src/main.gr": LIB,
                           ".grasm/registry/far/2.0.0/grasm.toml": manifest(name="far", version="2.0.0"),
                           ".grasm/registry/far/2.0.0/src/main.gr": LIB})
case("registry-ok", {"app/grasm.toml": manifest(deps=dep("far", version="^1.1.0")),
                     "app/src/main.gr": "import far\nfn main() {\n    print(far.f())\n}\n",
                     ".grasm/registry/far/1.0.0/grasm.toml": manifest(name="far", version="1.0.0"),
                     ".grasm/registry/far/1.0.0/src/main.gr": LIB,
                     ".grasm/registry/far/1.2.0/grasm.toml": manifest(name="far", version="1.2.0"),
                     ".grasm/registry/far/1.2.0/src/main.gr": LIB})

# One directory named two ways: through a symbolic link and by its real
# path, and relatively and absolutely. Rust asks the file system, so both
# are one package; `@ABS@` and `@LINK@` are filled in when written.
case("dep-symlink", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib") + dep("mid", path="../mid")),
                     "app/src/main.gr": "import lib\nimport mid\nfn main() {\n    print(lib.f() + mid.g())\n}\n",
                     "mid/grasm.toml": manifest(name="mid", deps=dep("lib", path="../liblink")),
                     "mid/src/main.gr": "import lib\npub fn g() -> i64 {\n    lib.f()\n}\n",
                     "lib/grasm.toml": manifest(name="lib"), "lib/src/main.gr": LIB,
                     "@LINK@liblink": "lib"})
case("dep-absolute", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib") + dep("mid", path="../mid")),
                      "app/src/main.gr": "import lib\nimport mid\nfn main() {\n    print(lib.f() + mid.g())\n}\n",
                      "mid/grasm.toml": manifest(name="mid", deps=dep("lib", path="@ABS@/lib")),
                      "mid/src/main.gr": "import lib\npub fn g() -> i64 {\n    lib.f()\n}\n",
                      "lib/grasm.toml": manifest(name="lib"), "lib/src/main.gr": LIB})
case("dep-symlink-app", {"real/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                         "real/src/main.gr": "import lib\nfn main() {\n    print(lib.f())\n}\n",
                         "lib/grasm.toml": manifest(name="lib"), "lib/src/main.gr": LIB,
                         "@LINK@app": "real"})

# The loader.
case("load-in-package", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                         "app/src/main.gr": "import lib.nothere\n" + MAIN,
                         "lib/grasm.toml": manifest(name="lib"), "lib/src/main.gr": LIB})
case("load-deep-in-package", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                              "app/src/main.gr": "import lib.sub.deep\n" + MAIN,
                              "lib/grasm.toml": manifest(name="lib"), "lib/src/main.gr": LIB})
case("load-undeclared", {"app/grasm.toml": manifest(deps=dep("mid", path="../mid")),
                         "app/src/main.gr": "import deep\n" + MAIN,
                         "mid/grasm.toml": manifest(name="mid", deps=dep("deep", path="../deep")), "mid/src/main.gr": LIB,
                         "deep/grasm.toml": manifest(name="deep"), "deep/src/main.gr": LIB})
case("load-in-dependency", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                            "app/src/main.gr": "import lib\n" + MAIN,
                            "lib/grasm.toml": manifest(name="lib"),
                            "lib/src/main.gr": "import helper.x\n" + LIB})
case("error-in-dependency", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                             "app/src/main.gr": "import lib\nfn main() {\n    print(lib.f())\n}\n",
                             "lib/grasm.toml": manifest(name="lib"),
                             "lib/src/main.gr": 'pub fn f() -> i64 {\n    "not a number"\n}\n'})
case("parse-error-in-dependency", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                                   "app/src/main.gr": "import lib\nfn main() {\n    print(lib.f())\n}\n",
                                   "lib/grasm.toml": manifest(name="lib"),
                                   "lib/src/main.gr": "pub fn f() -> i64 {\n    (1 +\n}\n"})

# The capability ceilings.
case("ceiling-sibling", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                         "app/src/main.gr": "import lib\nfn main() {\n    print(lib.f())\n}\n",
                         "lib/grasm.toml": manifest(name="lib", extra="capabilities = []"),
                         "lib/src/main.gr": LIB,
                         "lib/src/extra.gr": 'pub fn leak() {\n    print("x")\n}\n'})
case("ceiling-unsafe", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                        "app/src/main.gr": "import lib\nfn main() {\n    print(lib.f())\n}\n",
                        "lib/grasm.toml": manifest(name="lib", extra='capabilities = ["io"]'),
                        "lib/src/main.gr": "extern fn getpid() -> i32\n\npub fn f() -> i64 {\n    unsafe { getpid().as_i64() }\n}\n"})
case("ceiling-several", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib") + dep("net", path="../net")),
                         "app/src/main.gr": "import lib\nimport net\nfn main() {\n    print(lib.f() + net.g())\n}\n",
                         "lib/grasm.toml": manifest(name="lib", extra='capabilities = ["net", "net"]'),
                         "lib/src/main.gr": 'pub fn f() -> i64 {\n    print("a")\n    match proc_run("true", []) {\n        Ok(_) -> 1\n        Err(_) -> 0\n    }\n}\n',
                         "net/grasm.toml": manifest(name="net", extra="capabilities = []"),
                         "net/src/main.gr": 'pub fn g() -> i64 {\n    match tcp_connect("localhost", 1) {\n        Ok(_) -> 1\n        Err(_) -> 0\n    }\n}\n'})
case("ceiling-unknown", {"app/grasm.toml": manifest(deps=dep("lib", path="../lib")),
                         "app/src/main.gr": "import lib\nfn main() {\n    print(lib.f())\n}\n",
                         "lib/grasm.toml": manifest(name="lib", extra="capabilities = []"),
                         "lib/src/main.gr": LIB,
                         "lib/tests/broken.gr": "fn t( {\n"})


def main():
    out = sys.argv[1]
    entries = []
    for name, (files, entry) in sorted(CASES.items()):
        root = os.path.abspath(os.path.join(out, name))
        for rel, text in sorted(files.items()):
            if rel.startswith("@LINK@"):
                os.makedirs(root, exist_ok=True)
                os.symlink(text, os.path.join(root, rel[len("@LINK@"):]))
                continue
            path = os.path.join(root, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8", newline="") as f:
                f.write(text.replace("@ABS@", root))
        entries.append(os.path.join(root, entry))
    print("\n".join(entries))


if __name__ == "__main__":
    main()
