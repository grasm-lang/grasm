#!/usr/bin/env python3
"""Laws the answer has to obey, whichever engine produced it.

Every other generator here compares two engines. That cannot see a wrong
answer they share -- and they share a great deal: a bundle *is* the
interpreter with the program attached, and the interpreter is the oracle every
differential is measured against. Nothing checked the oracle.

So these check against arithmetic instead. `xs.rev()[0]` is the last element
or the implementation is wrong, and no second engine is consulted to find that
out. Each law prints `ok` when it holds, and it is run under `grasm run` and
under a `--native` build, so a law that fails in one names which.

A law a degenerate implementation satisfies is not a law. `rev` as the
identity function passes "rev twice is identity" -- identity twice is
identity -- and did: the first version of this file was written with that law
and no other, the interpreter's `reverse()` was removed as a test, and the
file reported everything holding. Half of these pin the answer rather than a
symmetry for that reason, and the pairs are kept together so the difference is
visible.

    scripts/fuzz_laws.py [--keep]
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

import importlib.util as _ilu


def _lib(name):
    spec = _ilu.spec_from_file_location(
        name,
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", f"{name}.py"),
    )
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


leaked = _lib("leaks").leaked
_lib("fresh").fresh_or_stop(GRASM)


PRELUDE = """type Bad = | Bad

effect Src {
    fn get() -> i64
}

fn say(ok: bool) / {io} {
    if ok {
        print("ok")
    } else {
        print("BROKEN")
    }
}

fn same[T](v: T) -> T {
    v
}

fn fails() -> i64 ! Bad {
    fail(Bad)
}

fn works() -> i64 ! Bad {
    4
}

"""

LAWS = {
 "rev twice is identity":      'let xs = [3, 1, 2]\n    say(xs.rev().rev() == xs)',
 # A law a degenerate implementation satisfies is not a law: `rev` as the
 # identity passes "rev twice is identity", and did.
 "rev actually reverses":      'let xs = [3, 1, 2]\n    let r = xs.rev()\n    say(r[0] == 2 and r[2] == 3 and r != xs)',
 "sort actually orders":       'var xs = [3, 1, 2]\n    xs.sort()\n    say(xs[0] == 1 and xs[1] == 2 and xs[2] == 3)',
 "sort is idempotent":         'var a = [3, 1, 2]\n    a.sort()\n    var b = a\n    b.sort()\n    say(a == b)',
 "sort preserves length":      'var xs = [3, 1, 2, 1]\n    let before = xs.len()\n    xs.sort()\n    say(xs.len() == before)',
 "concat adds lengths":        'let a = [1, 2]\n    let b = [3]\n    say((a + b).len() == a.len() + b.len())',
 "map preserves length":       'let xs = [1, 2, 3]\n    say(xs.map(|v| v * 2).len() == xs.len())',
 "filter actually filters":    'say([1, 2, 3, 4].filter(|v| v > 2) == [3, 4])',
 "filter partitions":          'let xs = [1, 2, 3, 4]\n    let a = xs.filter(|v| v % 2 == 0)\n    let b = xs.filter(|v| v % 2 != 0)\n    say(a.len() + b.len() == xs.len())',
 "fold sums":                  'let xs = [1, 2, 3]\n    say(xs.fold(0, |a, v| a + v) == xs.sum())',
 "range length":               'say((2..7).to_list().len() == 5)',
 "range map length":           'say((0..4).map(|v| v).len() == 4)',
 "map set then get":           'var m: Map[str, i64] = map()\n    m.set("k", 7)\n    say((m.get("k") ?? 0) == 7)',
 "map set twice keeps last":   'var m: Map[str, i64] = map()\n    m.set("k", 1)\n    m.set("k", 2)\n    say((m.get("k") ?? 0) == 2 and m.len() == 1)',
 "delete removes":             'var m: Map[str, i64] = map()\n    m.set("k", 1)\n    m.delete("k")\n    say(m.len() == 0 and m.get("k") == None)',
 "bytes round trip":           'let s = "hello"\n    say(s.to_bytes().to_str() == s)',
 "split join round trip":      'let s = "a,b,c"\n    say(s.split(",").join(",") == s)',
 "substr composes":            'let s = "abcdef"\n    say(s.substr(0, 3) + s.substr(3, 3) == s)',
 "len is chars":               'let s = "abc"\n    say(s.len() == s.chars().len())',
 "equality reflexive":         'let xs = [[1], [2, 3]]\n    say(xs == xs)',
 "equality symmetric":         'let a = [1, 2]\n    let b = [1, 2]\n    say((a == b) == (b == a))',
 "ordering antisymmetric":     'let a = 3\n    let b = 7\n    say((a < b) != (b < a))',
 "push then pop restores":     'var xs = [1, 2]\n    let before = xs.len()\n    xs.push(9)\n    let v = xs.pop()\n    say(xs.len() == before and (v ?? 0) == 9)',
 "remove shrinks by one":      'var xs = [1, 2, 3]\n    let before = xs.len()\n    let _v = xs.remove(1)\n    say(xs.len() == before - 1)',
 "repeat multiplies length":   'let s = "ab"\n    say(s.repeat(3).len() == s.len() * 3)',
 "abs is non-negative":        'say((0 - 7).abs() >= 0 and (7).abs() == 7)',
 "min max ordering":           'say((3).min(7) <= (3).max(7))',
 "trim is idempotent":         'let s = "  x  "\n    say(s.trim().trim() == s.trim())',
 "trim actually trims":        'say("  x  ".trim() == "x")',
 "upper lower stable":         'let s = "AbC"\n    say(s.to_upper().to_upper() == s.to_upper())',
 "upper actually uppers":      'say("AbC".to_upper() == "ABC" and "AbC".to_lower() == "abc")',
 "parse round trip":           'let n = 42\n    say((n.to_str().parse_int() ?? 0) == n)',
 "option unwrap default":      'let o: Option[i64] = None\n    say((o ?? 5) == 5 and (Some(3) ?? 5) == 3)',
 "list index matches order":   'let xs = [10, 20, 30]\n    say(xs[0] == 10 and xs[2] == 30)',
 # The areas the last two weeks of findings came from.
 "ref write is visible":       'let r = ref(1)\n    r.value = 9\n    let seen = r.value\n    release(r)\n    say(seen == 9)',
 "ref copies share a slot":    'let r = ref(1)\n    let c = r\n    c.value = 9\n    let seen = r.value\n    release(r)\n    say(seen == 9)',
 "released ref is not alive":  'let r = ref(1)\n    release(r)\n    say(alive(r) == false)',
 "handler answers its body":   'with Src handled_by SrcHandler{ get: || 7 } {\n        say(Src.get() == 7)\n    }',
 "inner handler wins":         'with Src handled_by SrcHandler{ get: || 1 } {\n        with Src handled_by SrcHandler{ get: || 2 } {\n            say(Src.get() == 2)\n        }\n    }',
 "handler restored after":     'with Src handled_by SrcHandler{ get: || 1 } {\n        with Src handled_by SrcHandler{ get: || 2 } {\n            let _v = Src.get()\n        }\n        say(Src.get() == 1)\n    }',
 "generic identity":           'say(same(7) == 7 and same("a") == "a")',
 "closure is stable":          'let held = 5\n    let f = || held\n    say(f() == f())',
 "map identity":               'let xs = [1, 2, 3]\n    say(xs.map(|v| v) == xs)',
 "map actually maps":          'say([1, 2].map(|v| v * 3) == [3, 6])',
 "filter all keeps all":       'let xs = [1, 2, 3]\n    say(xs.filter(|_v| true) == xs)',
 "filter none keeps none":     'let xs = [1, 2, 3]\n    say(xs.filter(|_v| false).len() == 0)',
 "par_map matches map":        'let xs = [1, 2, 3]\n    say(xs.par_map(|v| v * 2) == xs.map(|v| v * 2))',
 "fail is caught once":        'say((fails() ?? 0 - 1) == 0 - 1)',
 "ok passes through":          'say((works() ?? 0 - 1) == 4)',
 "keys and values align":      'var m: Map[str, i64] = map()\n    m.set("a", 1)\n    m.set("b", 2)\n    say(m.keys().len() == m.values().len() and m.keys().len() == m.len())',
}


def run(cmd, timeout=180):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="grasm_fuzz_laws_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, broken, rejected = 0, [], []
    try:
        for name, law in LAWS.items():
            row = "{io, async, Src}" if "par_map" in law else "{io, Src}"
            src = PRELUDE + "fn main() / " + row + " {\n    " + law + "\n}\n"
            with open(path, "w") as f:
                f.write(src)
            rc, out, err = run([GRASM, "run", path])
            if rc != 0:
                rejected.append(f"{name}: {(out + err).strip().splitlines()[0]}")
                continue
            checked += 1
            answers = {"run": out.strip()}
            rc2, o2, err2 = run([GRASM, "build", path, "--native", "-o", exe])
            if rc2 != 0:
                rejected.append(f"{name}: native declined")
                continue
            rc3, got, _ = run([exe])
            answers["native"] = got.strip()
            wrong = sorted(k for k, v in answers.items() if v != "ok")
            if wrong:
                broken.append(name)
                print(f"BROKEN {name} in {wrong}: {answers}\n{src}")
                continue
            held = leaked(exe)
            if held:
                broken.append(name)
                print(f"LEAKS {name}: {held}")
        for r in rejected:
            print(f"  NOT A LAW (the program does not run): {r}")
        print(f"laws: {checked}   broken: {len(broken)}   not run: {len(rejected)}")
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if broken or rejected else 0)


if __name__ == "__main__":
    main()
