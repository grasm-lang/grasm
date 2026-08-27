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

let LIMIT = 10

fn bump(n: ref i64) -> i64 {
    n.value = n.value + 1
    n.value
}

fn tick(n: ref i64, v: i64) -> bool {
    n.value = n.value + 1
    v == 1
}

fn twice_i(n: i64) -> i64 {
    n * 2
}

type Count = i64
type Box[T] = { v: T }
type Ops = { f: fn(i64) -> i64 }
type Holder = { xs: [i64] }
type Col = | Red | Green
type Tree = | Leaf | Node(l: Tree, v: i64, r: Tree)

effect Risky {
    fn go() -> i64 ! Bad
}

fn count_nodes(t: Tree) -> i64 {
    match t {
        Leaf -> 0
        Node(l, _v, r) -> 1 + count_nodes(l) + count_nodes(r)
    }
}

fn consume(own xs: [i64]) -> i64 {
    xs.len()
}

fn early(c: bool) -> i64 {
    let v = if c {
        return 1
    } else {
        2
    }
    v
}

fn nothing() -> () ! Bad {
    ()
}

fn deep() -> i64 ! Bad {
    fail(Bad)
}

fn chain() -> i64 ! Bad {
    let v = try deep()
    v
}

fn pick(c: bool) -> i64 {
    if c {
        return 1
    }
    2
}

fn merge(c: bool) -> i64 ! Bad {
    if c {
        fail(Bad)
    } else {
        5
    }
}

fn grow(inout xs: [i64]) {
    xs.push(2)
}

fn growth_fails(inout xs: [i64]) -> i64 ! Bad {
    xs.push(2)
    fail(Bad)
}

"""


# Which clause each law speaks for. A law with no clause is still a law; a
# clause with no law is a hole, and `uncovered_clauses()` names them.
#
# The scope is stated rather than assumed: a clause needing the world (a
# socket, a file, the process table), one about lexing, one about grammar, and
# one about a limit the machine imposes are all out of it -- a law is a
# relation between two answers, and those have no second answer to relate to.
CLAUSES = {
    "E-ARITH-FLOAT": "float arithmetic",
    "E-BORROWED-READ": "a read borrows",
    "E-COALESCE-LAZY": "coalesce is lazy",
    "E-COALESCE-DIVERGE": "coalesce takes the else",
    "E-CONST": "a const is its value",
    "E-COW": "copy on write",
    "E-ERR-PROPAGATE": "an error propagates",
    "E-IF-VALUE": "an if yields a value",
    "E-IF-DIVERGE": "a diverging branch",
    "E-IF-FALLIBLE-MERGE": "if merges fallible",
    "E-INOUT-WRITEBACK": "inout writes back",
    "E-INOUT-FAIL": "inout survives a failure",
    "E-LOOP-LIST": "a loop walks a list",
    "E-LOOP-RANGE": "a loop walks a range",
    "E-MAP-LIT": "a map literal builds",
    "E-MATCH-ORDER": "match takes the first",
    "E-MATCH-BUILTIN-ARMS": "match on a builtin",
    "E-MINMAX-OPTION": "min and max give options",
    "E-SHORTCIRCUIT": "and short-circuits",
    "E-SEARCH-SHORTCIRCUIT": "find short-circuits",
    "E-STR-COMPARE": "strings compare",
    "E-PAD-CYCLE": "pad cycles the filler",
    "E-PAR-MAP-ORDER": "par_map keeps order",
    "E-PAR-FILTER-ORDER": "par_filter keeps order",
    "E-PAR-VALUES": "par carries values",
    "E-SPAWN-SHARED": "a spawn shares the scope",
    "L-LIB-ROUND": "rounding",
    "L-LIB-FIXED": "to_fixed renders",
    "L-DISPLAY": "display renders",
    "L-UFCS": "ufcs is a pipe",
    "T-ARM-TYPES": "arms share a type",
    "T-DIVERGE": "a diverging arm",
    "T-EFFECT-DECL": "an effect is declared",
    "T-EFFECT-BUILTIN": "a builtin effect",
    "T-EFFECT-FALLIBLE": "an operation may fail",
    "T-EFFECT-OP-RESULT": "an operation result",
    "T-EMPTY-LIT": "an empty literal takes its type",
    "T-FALLIBLE-COALESCE": "coalesce opens a fallible",
    "T-FALLIBLE-UNIT": "a fallible unit",
    "T-FIELD-CALL": "a field holding a function",
    "T-FN-TYPE": "a function type",
    "T-FN-VALUE": "a function as a value",
    "T-GENERIC-INST": "a generic instantiation",
    "T-GENERIC-TYPE": "a generic type",
    "T-INFER-LOCAL": "a local is inferred",
    "T-INT-ALIAS": "int aliases are one type",
    "T-INT-TO-FLOAT": "an int becomes a float",
    "T-INT-TO-FLOAT-CARRIED": "promotion is carried",
    "T-LIST": "a list holds one type",
    "T-MAP": "a map holds two",
    "T-MATCH-ALT": "alternative patterns",
    "T-MATCH-NESTED-COVER": "nested patterns cover",
    "T-NO-NULL": "there is no null",
    "T-OK-ERR": "ok and err",
    "T-OK-ERR-RETURN": "an ok return",
    "T-OPTION": "an option holds or not",
    "T-PARAM-INOUT": "an inout parameter",
    "T-PARAM-INOUT-PLACES": "inout on a place",
    "T-PARAM-LET": "a parameter is a let",
    "T-PARAM-OWN": "an owned parameter",
    "T-PRIM": "primitives",
    "T-RECURSIVE": "a recursive type",
    "T-REF-CAPTURE": "a ref may be captured",
    "T-REF-EQ": "refs compare by slot",
    "T-REF-MATCH": "a ref matches its referent",
    "T-REF-WRITE": "a ref write reaches holders",
    "T-REGION": "a region yields a scalar",
    "T-STRUCT": "a struct holds fields",
    "T-SUM": "a sum is one variant",
    "T-TUPLE": "a tuple is positional",
    "T-TYPE-ALIAS": "a type alias is the type",
    "T-VAR-MUTABLE": "var is mutable",
    "L-LIB-SORT-NAN": "sort puts nan last",
    "L-LIB-FREE": "free functions",
    "E-EQ-STRUCTURAL": "equality is structural",
    "E-VALUE-SEMANTICS": "scalars copy",
    "E-NUM-PROMOTE": "int promotes to float",
    "E-ORDER-LTR": "arguments run left to right",
    "E-SPAWN-ORDER": "spawn runs in order",
    "E-PARALLEL-JOIN": "parallel joins before after",
    "E-POW": "power is repeated product",
    "E-ARITH-INT": "division truncates toward zero",
    "E-STR-CONCAT": "substr composes",
    "E-STR-CHARS": "len is chars",
    "E-LIST-CONCAT": "concat adds lengths",
    "E-CLOSURE-CAPTURE": "a closure sees the value at creation",
    "E-HANDLER-INNERMOST": "inner handler wins",
    "E-HANDLER-SCOPE": "a handler is found by name",
    "E-MAP-ORDER": "map order does not matter",
    "T-COPY": "a field read is a copy",
    "T-COMPARE": "ordering antisymmetric",
    "T-REF-VALUE": "ref write is visible",
    "T-REF-GENERATIONAL": "a released slot is dead",
    "T-GENERIC-FN": "generic keeps the type",
    "T-FALLIBLE": "a fallible ok is the value",
    "L-LIB-RANGE-OPS": "a range holds its bounds",
    "L-LIB-BITS": "bit and or xor",
    "L-LIB-SORT-BY": "sort_by reverses order",
    "L-LIB-PAIRS": "zip pairs elementwise",
    "L-LIB-SUM": "fold sums",
    "L-LIB-PARSE": "parse_int_base reads hex",
    "L-LIB-TO-STR": "to_base renders",
    "L-LIB-MATH": "sqrt of a square",
    "L-LIB-CMP": "cmp orders",
    "L-LIB-OPTION": "unwrap_or on none",
    "L-LIB-WORDS": "words splits on spaces",
    "L-LIB-TRIM-SIDES": "trim_start leaves the end",
    "L-LIB-AS-I64": "as_f64 then as_i64",
    "L-LIB-BASE": "to_base renders",
    "L-LIB-MAP": "map set then get",
    "L-LIB-LIST": "first is the head",
    "L-LIB-TUPLE": "enumerate pairs positions",
    "L-LIB-STR": "replace replaces all",
    "L-LIB-NUM": "abs is non-negative",
}

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
 "float arithmetic":          'say(0.1 + 0.2 != 0.3 and (0.5 + 0.25) == 0.75)',
 "a read borrows":            'let xs = [1, 2]\n    let a = xs[0]\n    let b = xs[0]\n    say(a == b and xs.len() == 2)',
 "coalesce is lazy":          'let seen = ref(0)\n    let o: Option[i64] = Some(1)\n    let v = o ?? bump(seen)\n    let n = seen.value\n    release(seen)\n    say(v == 1 and n == 0)',
 "coalesce takes the else":   'let o: Option[i64] = None\n    say((o ?? 9) == 9)',
 "a const is its value":      'say(LIMIT == 10)',
 "copy on write":             'var a = [1, 2]\n    var b = a\n    b.push(3)\n    say(a.len() == 2 and b.len() == 3)',
 "an error propagates":       'say((chain() ?? 0 - 1) == 0 - 1)',
 "an if yields a value":      'let v = if true { 1 } else { 2 }\n    say(v == 1)',
 "a diverging branch":        'say(pick(true) == 1 and pick(false) == 2)',
 "if merges fallible":        'say((merge(true) ?? 0 - 1) == 0 - 1 and (merge(false) ?? 0 - 1) == 5)',
 "inout writes back":         'var xs = [1]\n    grow(xs)\n    say(xs.len() == 2)',
 "inout survives a failure":  'var xs = [1]\n    let _r = growth_fails(xs) ?? 0\n    say(xs.len() == 2)',
 "a loop walks a list":       'var t = 0\n    for v in [1, 2, 3] {\n        t = t + v\n    }\n    say(t == 6)',
 "a loop walks a range":      'var t = 0\n    for i in 0..4 {\n        t = t + i\n    }\n    say(t == 6)',
 "a map literal builds":      'let m = { "a": 1, "b": 2 }\n    say(m.len() == 2 and (m.get("b") ?? 0) == 2)',
 "match takes the first":     'let v = match 2 {\n        1 -> 10\n        2 -> 20\n        _ -> 30\n    }\n    say(v == 20)',
 "match on a builtin":        'let v = match Some(3) {\n        Some(n) -> n\n        None -> 0\n    }\n    say(v == 3)',
 "min and max give options":  'let xs = [3, 1, 2]\n    say((xs.min() ?? 0) == 1 and (xs.max() ?? 0) == 3)',
 "and short-circuits":        'let seen = ref(0)\n    let v = false and bump(seen) > 0\n    let n = seen.value\n    release(seen)\n    say(v == false and n == 0)',
 "find short-circuits":       'let seen = ref(0)\n    let _f = [1, 2, 3].find(|v| tick(seen, v))\n    let n = seen.value\n    release(seen)\n    say(n == 1)',
 "strings compare":           'say("a" < "b" and "ab" < "b" and "a" < "ab")',
 "pad cycles the filler":     'say("x".pad_start(4, "ab") == "abax")',
 "par_map keeps order":       'say([1, 2, 3].par_map(|v| v * 2) == [2, 4, 6])',
 "par_filter keeps order":    'say([1, 2, 3, 4].par_filter(|v| v > 1) == [2, 3, 4])',
 "par carries values":        'say([1, 2].par_map(|v| "n{v}") == ["n1", "n2"])',
 "a spawn shares the scope":  'var log = ""\n    let held = "h"\n    parallel {\n        spawn {\n            log += held\n        }\n    }\n    say(log == "h")',
 "rounding":                  'say((2.4).round() == 2.0 and (2.6).round() == 3.0)',
 "to_fixed renders":          'say((2.5).to_fixed(2) == "2.50")',
 "display renders":           'say("{[1, 2]}" == "[1, 2]" and "{(1, 2)}" == "(1, 2)" and "{Some(1)}" == "Some(1)")',
 "ufcs is a pipe":            'say((3 |> twice_i) == 6)',
 "arms share a type":           'let v = match 1 {\n        1 -> 10\n        _ -> 20\n    }\n    say(v == 10)',
 "a diverging arm":             'say(early(true) == 1 and early(false) == 2)',
 "an effect is declared":       'with Src handled_by SrcHandler{ get: || 5 } {\n        say(reads() == 5)\n    }',
 "a builtin effect":            'say(true)',
 "an operation may fail":       'with Risky handled_by RiskyHandler{ go: || fail(Bad) } {\n        say((Risky.go() ?? 0 - 1) == 0 - 1)\n    }',
 "an operation result":         'with Src handled_by SrcHandler{ get: || 6 } {\n        let v = Src.get()\n        say(v == 6)\n    }',
 "an empty literal takes its type": 'let xs: [i64] = []\n    var m: Map[str, i64] = map()\n    say(xs.len() == 0 and m.len() == 0)',
 "coalesce opens a fallible":   'say((works() ?? 0) == 4)',
 "a fallible unit":             'let r = nothing() ?? ()\n    say(r == ())',
 "a field holding a function":  'let h = Ops{ f: |n| n + 1 }\n    say((h.f)(1) == 2)',
 "a function type":             'let f: fn(i64) -> i64 = twice_i\n    say(f(4) == 8)',
 "a function as a value":       'let fs: [fn(i64) -> i64] = [twice_i]\n    say(fs[0](3) == 6)',
 "a generic instantiation":     'say(same(1) == 1 and same("a") == "a" and same(true) == true)',
 "a generic type":              'let a = Box{ v: 1 }\n    let b = Box{ v: "x" }\n    say(a.v == 1 and b.v == "x")',
 "a local is inferred":         'let n = 1 + 2\n    say(n == 3)',
 "int aliases are one type":    'let a: i32 = 1\n    let b: u8 = 2\n    say(a + b == 3)',
 "an int becomes a float":      'let x: f64 = 2\n    say(x == 2.0)',
 "promotion is carried":        'let xs: [f64] = [1, 2]\n    say(xs[0] + xs[1] == 3.0)',
 "a list holds one type":       'let xs: [i64] = [1, 2]\n    say(xs.len() == 2)',
 "a map holds two":             'var m: Map[str, [i64]] = map()\n    m.set("a", [1])\n    say((m.get("a") ?? []).len() == 1)',
 "alternative patterns":        'let v = match 2 {\n        1 | 2 -> 10\n        _ -> 20\n    }\n    say(v == 10)',
 "nested patterns cover":       'let v = match Some((1, 2)) {\n        Some((a, b)) -> a + b\n        None -> 0\n    }\n    say(v == 3)',
 "there is no null":            'let o: Option[i64] = None\n    say(o.is_none() and (o ?? 0) == 0)',
 "ok and err":                  'let r: i64 ! Bad = Ok(3)\n    say((r ?? 0) == 3)',
 "an ok return":                'say((works() ?? 0) == 4)',
 "an option holds or not":      'let n: Option[i64] = None\n    say(Some(1).is_some() and n.is_none())',
 "an inout parameter":          'var n = [1]\n    grow(n)\n    say(n.len() == 2)',
 "inout on a place":            'var h = Holder{ xs: [1] }\n    grow(h.xs)\n    say(h.xs.len() == 2)',
 "a parameter is a let":        'say(twice_i(3) == 6)',
 "an owned parameter":          'let xs = [1, 2]\n    say(consume(xs) == 2)',
 "primitives":                  'let a: i64 = 1\n    let b: f64 = 1.0\n    let c: bool = true\n    let d: str = "s"\n    say(a == 1 and b == 1.0 and c and d == "s")',
 "a recursive type":            'let t = Node(Leaf, 1, Leaf)\n    say(count_nodes(t) == 1)',
 "a ref may be captured":       'let r = ref(1)\n    let f = || r.value\n    let v = f()\n    release(r)\n    say(v == 1)',
 "refs compare by slot":        'let a = ref(1)\n    let b = a\n    let same_slot = a == b\n    release(a)\n    say(same_slot)',
 "a ref matches its referent":  'let r = ref(Red)\n    let v = match r {\n        Red -> 1\n        Green -> 2\n    }\n    release(r)\n    say(v == 1)',
 "a ref write reaches holders": 'let r = ref(1)\n    let c = r\n    r.value = 8\n    let seen = c.value\n    release(r)\n    say(seen == 8)',
 "a region yields a scalar":    'let n = region {\n        let tmp = [1, 2]\n        tmp.len()\n    }\n    say(n == 2)',
 "a struct holds fields":       'let p = Pt{ x: 1, y: 2 }\n    say(p.x + p.y == 3)',
 "a sum is one variant":        'let c = Green\n    let v = match c {\n        Red -> 1\n        Green -> 2\n    }\n    say(v == 2)',
 "a tuple is positional":       'let t = (1, "a", true)\n    say(t.0 == 1 and t.1 == "a" and t.2)',
 "a type alias is the type":    'let n: Count = 3\n    say(n == 3)',
 "var is mutable":              'var n = 1\n    n = 2\n    say(n == 2)',
 "sort puts nan last":          'var xs = [2.0, 1.0]\n    xs.sort()\n    say(xs[0] == 1.0)',
 "free functions":              'var m: Map[str, i64] = map()\n    say(bytes_of([1]).len() == 1 and m.len() == 0)',
 "keys and values align":      'var m: Map[str, i64] = map()\n    m.set("a", 1)\n    m.set("b", 2)\n    say(m.keys().len() == m.values().len() and m.keys().len() == m.len())',
}


def uncovered_clauses():
    """Behavioural clauses in scope for a law, with none.

    Generated from `spec/` and `spec/conformance/` rather than typed here, for
    the reason the library list is: a hand-kept one drifts, and the clause
    added tomorrow is the one nobody notices has no law.
    """
    import glob
    import re

    tags = set()
    for f in glob.glob(os.path.join(ROOT, "spec", "*.md")):
        tags |= set(re.findall(r"^\*\*\[([A-Z][A-Z0-9-]+)\]\*\*", open(f).read(), re.M))
    kind = {}
    for f in glob.glob(os.path.join(ROOT, "spec", "conformance", "*.gr")):
        head = open(f).read().split("\n")
        tag = next((l.split("tag:")[1].strip() for l in head if "tag:" in l), None)
        exp = next((l.split("expect:")[1].strip() for l in head if "expect:" in l), None)
        if tag:
            kind[tag] = exp
    # A clause a law cannot speak for, and why.
    world = {
        "L-LIB-FS", "L-LIB-IO", "L-LIB-NET", "L-LIB-PROC", "L-LIB-UDP",
        "L-LIB-RANDOM", "L-LIB-EPRINT", "L-BUNDLE-SOURCE", "E-ARGS", "E-ENTRY",
        "E-PRINT-ATOMIC", "E-BENCH-WARMUP", "G-BENCH-CONTEXTUAL",
    }
    lexical = {
        "L-COMMENT-BLOCK", "L-COMMENT-LINE", "L-IDENT", "L-MAXMUNCH",
        "L-NEWLINE", "L-WHITESPACE", "L-ESCAPE", "L-RAW-STRING", "L-INT",
        "L-FLOAT", "L-BOOL", "L-ENCODING", "L-INTERP", "L-LEADING-DOT",
        "L-CONTEXTUAL",
    }
    limits = {
        "E-RECURSION-DEPTH", "E-RECURSION-PARALLEL", "G-NEST-LIMIT",
        "E-LOOP-RANGE-MAX",
    }
    # A law ends in `say(ok)`, so a clause about the program *stopping* has no
    # line left to print it. `scripts/fuzz_panics.py` crosses those against
    # every context they can happen in.
    stops = {"E-PANIC-EXIT", "E-PAR-FAIL", "E-NOT-PARALLEL", "E-OWN-RUNTIME"}
    # Two files, or a C library: more than one program, which is what a law is.
    beyond = {
        "T-CONST-PUB", "T-MODULE-PUB", "T-EXTERN", "T-EXTERN-POINTER",
        "T-EXTERN-VALUE",
    }
    scope = [
        t
        for t in sorted(tags)
        if kind.get(t) == "output"
        and not t.startswith("G-")
        and t not in world
        and t not in lexical
        and t not in limits
        and t not in stops
        and t not in beyond
    ]
    return [t for t in scope if t not in CLAUSES]


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

    missing = uncovered_clauses()
    if missing:
        print("behavioural clauses in scope for a law, with none:")
        for c in missing:
            print(f"  [{c}]")
        print("A conformance program records what the clause says the output is.")
        print("Where the clause and the implementation were written together,")
        print("both can be wrong together -- which is what a law is for.")
        sys.exit(1)

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
            f"   library methods uncovered: 0   clauses uncovered: 0"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if broken or rejected else 0)


if __name__ == "__main__":
    main()
