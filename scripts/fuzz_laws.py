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

type Pt = { x: i64, y: i64 }

fn reads() -> i64 / {Src} {
    Src.get()
}

fn note(seen: ref str, tag: str) -> i64 {
    seen.value = seen.value + tag
    1
}

fn two(a: i64, b: i64) -> i64 {
    a + b
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
 "bit and or xor":             'let a = 12\n    let b = 10\n    say(a.bit_and(b) == 8 and a.bit_or(b) == 14 and a.bit_xor(b) == 6)',
 "bit not is complement":      'say((0).bit_not() == 0 - 1 and (0 - 1).bit_not() == 0)',
 "shifts scale by two":        'say((1).shl(3) == 8 and (8).shr(3) == 1)',
 "ushr fills with zero":       'say((0 - 1).ushr(63) == 1)',
 "xor is its own inverse":     'let a = 12\n    let k = 10\n    say(a.bit_xor(k).bit_xor(k) == a)',
 "as_f64 then as_i64":         'say((7).as_f64() == 7.0 and (7.9).as_i64() == 7)',
 "sqrt of a square":           'say((9.0).sqrt() == 3.0)',
 "floor and ceil bracket":     'let x = 2.5\n    say(x.floor() == 2.0 and x.ceil() == 3.0)',
 "cbrt of a cube":             'say((27.0).cbrt() == 3.0)',
 "exp and ln invert":          'let x = 2.0\n    say((x.exp().ln() - x).abs() < 0.000001)',
 "log2 of a power":            'say((8.0).log2() == 3.0)',
 "log with a base":            'say(((81.0).log(3.0) - 4.0).abs() < 0.000001)',
 "sin of zero":                'say((0.0).sin() == 0.0 and (0.0).asin() == 0.0)',
 "sinh of zero":               'say((0.0).sinh() == 0.0)',
 "atan2 quarter turn":         'let q = (1.0).atan2(0.0)\n    say((q - (90.0).to_radians()).abs() < 0.000001)',
 "cmp orders":                 'say((1).cmp(2) < 0 and (2).cmp(1) > 0 and (2).cmp(2) == 0)',
 "to_base renders":            'say((255).to_base(16) == "ff" and (5).to_base(2) == "101")',
 "is_nan is false for a number": 'say((1.0).is_nan() == false and (1.0).is_finite())',
 "byte_len counts bytes":      'say("abc".byte_len() == 3)',
 "code_at and codes agree":    'let s = "abc"\n    say(s.code_at(0) == s.codes()[0])',
 "to_char inverts code_at":    'let s = "abc"\n    say(s.code_at(1).to_char() == "b")',
 "contains and index_of":      'let s = "hello"\n    say(s.contains("ll") and (s.index_of("ll") ?? 0 - 1) == 2)',
 "index_of misses":            'say("hello".index_of("zz") == None)',
 "lines splits on newline":    'say("a\\nb".lines().len() == 2)',
 "words splits on spaces":     'say("a b  c".words().len() == 3)',
 "pad_start reaches width":    'let p = "7".pad_start(3, "0")\n    say(p == "007" and p.len() == 3)',
 "replace replaces all":       'say("aXbXc".replace("X", "-") == "a-b-c")',
 "trim_start leaves the end":  'say("  x  ".trim_start() == "x  ")',
 "parse_float round trips":    'say(((2.5).to_str().parse_float() ?? 0.0) == 2.5)',
 "parse_int_base reads hex":   'say(("ff".parse_int_base(16) ?? 0) == 255)',
 "concat joins bytes":         'let a = bytes_of([1])\n    let b = bytes_of([2])\n    say(a.concat(b).len() == 2 and a.concat(b).at(1) == 2)',
 "slice takes a window":       'let b = bytes_of([1, 2, 3])\n    say(b.slice(1, 2).to_list() == [2, 3])',
 "at reads in order":          'let b = bytes_of([7, 8])\n    say(b.at(0) == 7 and b.at(1) == 8)',
 "first is the head":          'let xs = [4, 5, 6]\n    say((xs.first() ?? 0) == 4 and (xs.last() ?? 0) == 6)',
 "first of empty is none":     'let xs: [i64] = []\n    say(xs.first() == None and xs.is_empty())',
 "enumerate pairs positions":  'let e = [7, 8].enumerate()\n    say(e.len() == 2 and e[1].0 == 1 and e[1].1 == 8)',
 "zip pairs elementwise":      'let z = [1, 2].zip(["a", "b"])\n    say(z.len() == 2 and z[0].1 == "a")',
 "zip stops at the shorter":   'say([1, 2, 3].zip(["a"]).len() == 1)',
 "any and all agree":          'let xs = [2, 4]\n    say(xs.all(|v| v % 2 == 0) and xs.any(|v| v == 4) and xs.any(|v| v == 9) == false)',
 "find locates":               'say(([1, 2, 3].find(|v| v > 1) ?? 0) == 2)',
 "sort_by reverses order":     'var xs = [1, 3, 2]\n    xs.sort_by(|v| 0 - v)\n    say(xs[0] == 3 and xs[2] == 1)',
 "par_filter matches filter":  'let xs = [1, 2, 3, 4]\n    say(xs.par_filter(|v| v % 2 == 0) == xs.filter(|v| v % 2 == 0))',
 "entries match keys":         'var m: Map[str, i64] = map()\n    m.set("a", 1)\n    say(m.entries().len() == 1 and m.has("a") and m.is_empty() == false)',
 "expect gives the payload":   'say(Some(3).expect("there") == 3 and Some(3).is_none() == false)',
 "unwrap_or on none":          'let o: Option[i64] = None\n    say(o.unwrap_or(9) == 9 and Some(1).unwrap_or(9) == 1)',
 # The language's own semantics. A conformance program records what a clause
 # says the output is; if the clause and the implementation were written
 # together, both can be wrong together. These state a relation instead.
 "equality ignores field order": 'say(Pt{ x: 1, y: 2 } == Pt{ y: 2, x: 1 })',
 "equality is structural":     'say([[1], [2, 3]] == [[1], [2, 3]] and Pt{ x: 1, y: 2 } != Pt{ x: 1, y: 3 })',
 "built two ways is equal":    'var a: [i64] = []\n    a.push(1)\n    a.push(2)\n    say(a == [1, 2])',
 "map order does not matter":  'var m: Map[str, i64] = map()\n    m.set("a", 1)\n    m.set("b", 2)\n    var n: Map[str, i64] = map()\n    n.set("b", 2)\n    n.set("a", 1)\n    say(m == n)',
 "scalars copy":               'var a = 1\n    var b = a\n    b = 9\n    say(a == 1)',
 "a field read is a copy":     'let p = Pt{ x: 1, y: 2 }\n    var c = p.x\n    c = 9\n    say(p.x == 1)',
 "int promotes to float":      'let xs: [f64] = [1, 2.5]\n    say(xs[0] == 1.0)',
 "promotion is order-free":    'let a: [f64] = [1, 2.5]\n    let b: [f64] = [2.5, 1]\n    say(a[0] == b[1] and a[1] == b[0])',
 "arguments run left to right": 'let seen = ref("")\n    let _v = two(note(seen, "a"), note(seen, "b"))\n    let order = seen.value\n    release(seen)\n    say(order == "ab")',
 "spawn runs in order":        'var log = ""\n    parallel {\n        spawn {\n            log += "a"\n        }\n        spawn {\n            log += "b"\n        }\n    }\n    say(log == "ab")',
 "parallel joins before after": 'var log = ""\n    parallel {\n        spawn {\n            log += "a"\n        }\n    }\n    log += "z"\n    say(log == "az")',
 "power is repeated product":  'say((2 ^ 10) == 1024)',
 "negative power is a fraction": 'say((2.0 ^ (0.0 - 1.0)) == 0.5)',
 "division truncates toward zero": 'say((7 / 2) == 3 and ((0 - 7) / 2) == 0 - 3)',
 "remainder matches division": 'let a = 7\n    let b = 2\n    say(a / b * b + a % b == a)',
 "a released slot is dead":    'let r = ref(1)\n    let c = r\n    release(r)\n    say(alive(c) == false)',
 "a double release is safe":   'let r = ref(1)\n    release(r)\n    release(r)\n    say(alive(r) == false)',
 "a range holds its bounds":   'let r = 2..5\n    say(r.to_list()[0] == 2 and r.len() == 3)',
 "inclusive adds one":         'say((0..3).len() + 1 == (0..=3).len())',
 "a closure sees the value at creation": 'var n = 1\n    let f = || n\n    n = 9\n    say(f() == 1)',
 "a handler is found by name": 'with Src handled_by SrcHandler{ get: || 3 } {\n        say(reads() == 3)\n    }',
 "generic keeps the type":     'say(same([1, 2]) == [1, 2] and same(Pt{ x: 1, y: 2 }) == Pt{ x: 1, y: 2 })',
 "a fallible ok is the value": 'say((works() ?? 0) == 4 and (fails() ?? 7) == 7)',
 "unit has one value":         'let a = ()\n    let b = ()\n    say(a == b)',
 "keys and values align":      'var m: Map[str, i64] = map()\n    m.set("a", 1)\n    m.set("b", 2)\n    say(m.keys().len() == m.values().len() and m.keys().len() == m.len())',
}


def uncovered():
    """Library methods with no law.

    Generated from `spec/library.md` rather than typed here, because a
    hand-kept list drifts: the specification names the surface, and a method
    added there without a law would otherwise be a hole nobody sees. The
    coverage is the point of this file -- a differential cannot vouch for any
    of these, so the only check they get is the one below.
    """
    import re

    spec = open(os.path.join(ROOT, "spec", "library.md")).read()
    named = sorted(set(re.findall(r"^\| `[a-z]+\.([a-z_0-9]+)\(", spec, re.M)))
    body = "\n".join(LAWS.values())
    return [m for m in named if not re.search(r"\." + re.escape(m) + r"\(", body)]


def run(cmd, timeout=180):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    gaps = uncovered()
    if gaps:
        print("library methods with no law -- every one is a wrong answer that")
        print("no differential here could see:")
        for m in gaps:
            print(f"  .{m}()")
        sys.exit(1)

    work = tempfile.mkdtemp(prefix="grasm_fuzz_laws_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, broken, rejected = 0, [], []
    try:
        for name, law in LAWS.items():
            # Anything that performs `async` needs the row, and a law written
            # without it does not run at all -- the fault F58 found, which is
            # why "not run" is a failure here rather than a count.
            needs_async = any(
                w in law for w in ("par_map", "par_filter", "parallel", "spawn")
            )
            row = "{io, async, Src}" if needs_async else "{io, Src}"
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
        print(
            f"laws: {checked}   broken: {len(broken)}   not run: {len(rejected)}"
            f"   library methods uncovered: 0"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if broken or rejected else 0)


if __name__ == "__main__":
    main()
