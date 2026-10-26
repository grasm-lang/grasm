#!/usr/bin/env python3
"""An incremental native build is the same build as a cold one.

`grasm build --native` caches one object per module in `.grasm-cache` and
reuses it while its key holds: the module's source, the program's signatures,
the compiler, and `layout_fingerprint` -- the decisions made over the whole
program that one module's code depends on. Four times now a decision was
missing from the key, and each time an edit to one module left another
module's stale object in the binary: a constant's value, a struct turning
into a value struct, the compiler itself, and a function becoming recursive
(it then takes a hidden stack-floor argument, and the reused caller passed
its first argument there). None of them showed in any gate, because every
gate builds into a fresh directory, where the cache is always cold.

Each scenario below is two versions of a small program that differ in one
module. The gate builds version one, then version two over it in the same
directory (the warm cache), then version two alone in an empty directory
(the cold one) under the same file name, since the ad-hoc signature names
the file. The two binaries must be byte-identical, both must print and exit
as `grasm run` does, and then the same in the other direction. A rebuild
that changes nothing must write no object, or the cache is not a cache.

The control: after the warm build of version two, an object of version one
is put in place of a module's fresh object, as a missing key input would,
and the comparison must notice.
"""
import os
import shutil
import subprocess
import sys
import tempfile

GRASM = os.environ.get("GRASM", "./target/release/grasm")

# Each: {"v1": {file: text}, "v2": {file: text}, "stale": the module whose
# version-one object the control plants}. Only `helper.gr` or `main.gr`
# changes between the versions, never both.
SCENARIOS = {
    # [E-CONST]: one value for the whole program, read into the importer.
    "constant": {
        "v1": {
            "helper.gr": "pub let K = 10\n\npub fn f() -> i64 {\n    5\n}\n",
            "main.gr": 'import helper\n\nfn main() {\n    print("{helper.K} {helper.f()}")\n}\n',
        },
        "v2": {
            "helper.gr": "pub let K = 999\n\npub fn f() -> i64 {\n    5\n}\n",
        },
        "stale": "main",
    },
    # A struct captured by a closure stays boxed; one only read is a value.
    "layout": {
        "v1": {
            "helper.gr": 'type TabH = { xs: [str], ys: [str] }\n\npub fn make() -> TabH {\n    return TabH{ xs: ["a", "b", "c"], ys: ["d"] }\n}\n\npub fn has(t: TabH, s: str) -> bool {\n    return t.xs.contains(s)\n}\n',
            "main.gr": 'import helper\n\nfn main() {\n    let t = helper.make()\n    print(["a", "z"].map(|s| if helper.has(t, s) { "y" } else { "n" }).join(","))\n}\n',
        },
        "v2": {
            "main.gr": 'import helper\n\nfn main() {\n    let t = helper.make()\n    print("{t.xs.len()} {t.ys.len()}")\n}\n',
        },
        "stale": "helper",
    },
    # Recursion is part of a function's ABI (the stack-floor argument).
    "recursion": {
        "v1": {
            "helper.gr": "pub fn f(n: i64) -> i64 {\n    n + 1\n}\n",
            "main.gr": 'import helper\n\nfn main() {\n    print("{helper.f(5)} {helper.f(40)}")\n}\n',
        },
        "v2": {
            "helper.gr": "pub fn f(n: i64) -> i64 {\n    if n <= 0 {\n        1\n    } else {\n        f(n - 1) + 1\n    }\n}\n",
        },
        "stale": "main",
    },
    # The shape of the case that was found. A function that names another
    # without calling it may be handing it on as a value, so the recursion
    # analysis gives it the floor -- and it matches names against every
    # function in the program, unqualified. `main` gaining a function called
    # `seen` makes the helper's `count`, whose local is called `seen`, take
    # the floor: an edit to one module changing the ABI of a function in
    # another, whose source did not move.
    "name-clash": {
        "v1": {
            "helper.gr": "pub fn count(xs: [i64]) -> i64 {\n    var seen: Map[i64, bool] = {}\n    for x in xs {\n        seen.set(x, true)\n    }\n    seen.len()\n}\n",
            "main.gr": 'import helper\n\nfn main() {\n    print("{helper.count([1, 2, 2, 3])}")\n}\n',
        },
        "v2": {
            "main.gr": 'import helper\n\nfn seen() -> i64 {\n    7\n}\n\nfn main() {\n    print("{helper.count([1, 2, 2, 3])} {seen()}")\n}\n',
        },
        "stale": "helper",
    },
    # Generic functions instantiated at new types by the other module.
    "generic": {
        "v1": {
            "helper.gr": "pub fn id[T](x: T) -> T {\n    x\n}\n\npub fn pair[T](x: T) -> [T] {\n    [x, x]\n}\n",
            "main.gr": 'import helper\n\nfn main() {\n    print("{helper.id(5)} {helper.pair(1)}")\n}\n',
        },
        "v2": {
            "main.gr": 'import helper\n\nfn main() {\n    print("{helper.id(5)} {helper.pair(1)} {helper.id("s")} {helper.pair(2.5)} {helper.pair("q")}")\n}\n',
        },
        "stale": "main",
    },
}


def sh(args, cwd=None, timeout=300):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout)


def write(d, files):
    for name, text in files.items():
        with open(os.path.join(d, name), "w") as f:
            f.write(text)


def build(d):
    r = sh([GRASM, "build", os.path.join(d, "main.gr"), "--native", "-o", os.path.join(d, "app")])
    if r.returncode != 0:
        raise RuntimeError(f"build in {d} failed: {r.stderr.strip()[:300]}")


def cache_objects(d):
    c = os.path.join(d, ".grasm-cache")
    return sorted(os.listdir(c)) if os.path.isdir(c) else []


def behaviour(d, native):
    if native:
        r = sh([os.path.join(d, "app")])
    else:
        r = sh([GRASM, "run", os.path.join(d, "main.gr")])
    return (r.returncode, r.stdout, r.stderr)


def same_bytes(a, b):
    with open(a, "rb") as fa, open(b, "rb") as fb:
        return fa.read() == fb.read()


def one_way(work, name, before, after, label, problems):
    """Build `before` then `after` in one directory, `after` alone in another."""
    warm = os.path.join(work, f"{name}-{label}-warm")
    cold = os.path.join(work, f"{name}-{label}-cold")
    os.makedirs(warm)
    os.makedirs(cold)
    write(warm, before)
    build(warm)
    write(warm, after)
    build(warm)
    write(cold, after)
    build(cold)
    ok = True
    if not same_bytes(os.path.join(warm, "app"), os.path.join(cold, "app")):
        problems.append(f"{name} ({label}): the warm-cache build differs from the cold one")
        ok = False
    want = behaviour(cold, native=False)
    for where, d in (("warm", warm), ("cold", cold)):
        got = behaviour(d, native=True)
        if got != want:
            problems.append(
                f"{name} ({label}): the {where} build exits {got[0]} printing {got[1]!r}; "
                f"grasm run exits {want[0]} printing {want[1]!r}"
            )
            ok = False
    # A rebuild of what is already built writes no object.
    objs = cache_objects(warm)
    build(warm)
    if cache_objects(warm) != objs:
        problems.append(f"{name} ({label}): a rebuild that changed nothing wrote new objects")
        ok = False
    return ok, warm


def control(work, name, sc, problems):
    """Plant version one's object for a module under version two's key."""
    d = os.path.join(work, f"{name}-control")
    os.makedirs(d)
    v1 = dict(sc["v1"])
    v2 = dict(v1, **sc["v2"])
    write(d, v1)
    build(d)
    module = sc["stale"]
    old = [o for o in cache_objects(d) if o.startswith(module + "-")]
    if len(old) != 1:
        problems.append(f"{name} (control): expected one cached object for `{module}`, found {old}")
        return False
    keep = os.path.join(work, f"{name}-control.o")
    shutil.copy(os.path.join(d, ".grasm-cache", old[0]), keep)
    write(d, v2)
    build(d)
    fresh = [o for o in cache_objects(d) if o.startswith(module + "-") and o != old[0]]
    if len(fresh) != 1:
        # The module's key did not change, so there is nothing to plant over;
        # the scenario then relies on the comparison alone.
        problems.append(f"{name} (control): `{module}`'s key did not change between the versions")
        return False
    with open(keep, "rb") as f, open(os.path.join(d, ".grasm-cache", fresh[0]), "wb") as g:
        g.write(f.read())
    os.remove(os.path.join(d, "app"))
    build(d)
    cold = os.path.join(work, f"{name}-control-cold")
    os.makedirs(cold)
    write(cold, v2)
    build(cold)
    return not same_bytes(os.path.join(d, "app"), os.path.join(cold, "app"))


def main():
    work = tempfile.mkdtemp(prefix="inctest")
    problems = []
    passed = 0
    caught = 0
    try:
        for name, sc in SCENARIOS.items():
            v1 = dict(sc["v1"])
            v2 = dict(v1, **sc["v2"])
            ok1, _ = one_way(work, name, v1, v2, "forward", problems)
            ok2, _ = one_way(work, name, v2, v1, "back", problems)
            if ok1 and ok2:
                passed += 1
            if control(work, name, sc, problems):
                caught += 1
            else:
                problems.append(f"{name} (control): a stale `{sc['stale']}` object went unnoticed")
    finally:
        shutil.rmtree(work, ignore_errors=True)
    for p in problems:
        print(p)
    total = len(SCENARIOS)
    print(f"scenarios: {total}   warm and cold builds identical both ways: {passed}   stale objects planted: {total}   caught: {caught}")
    sys.exit(0 if passed == total and caught == total and not problems else 1)


if __name__ == "__main__":
    main()
