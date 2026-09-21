#!/usr/bin/env python3
"""Every way out of every block that owns something.

The generators build programs that fall out of their blocks normally. All
twelve of them do, which is why four faults in a single construct -- F59,
F60, F61, F62 -- were all found by hand and none by a gate.

A block that owns cleanup has one path where the cleanup is written and
several where it is not. `with` owns an installed handler and its captures;
a loop owns its scope; a `region` owns a Tier-1 arena; a plain block and an
`unsafe` block own their locals. `fail`, `return`, `break` and `continue`
each leave without reaching the end, and each one is a place the cleanup can
be missed -- or, if the construct does not report that it left, a place the
next statement is written into a block already closed (F60, F63).

So the axis is the construct crossed with the way out, and each program
holds something boxed while it leaves, because a leak is what a missed
release looks like and the output is right either way.

    scripts/fuzz_exits.py [--keep]
"""

import argparse
import os
import re
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
# One binary, rebuilt in place, instead of one file per program (see the module).
build_native = _lib("inplace").build
_lib("fresh").fresh_or_stop(GRASM)


PRELUDE = """type Bad = | Bad

type Pair = { a: [i64], b: [i64] }

effect Src {
    fn get() -> i64
}

// A `try` is an early exit the reader does not see: the branch that leaves is
// written nowhere, and a block that owns cleanup has to release it on a path
// no line of the source names. Every other exit here is spelled out.
fn risky() -> i64 ! Bad {
    fail(Bad)
}

// Generic, so one spelling serves an element that must be an `i64` and one
// that must be a list.
fn risky_of[T](v: T) -> T ! Bad {
    fail(Bad)
}
"""

# The block, and what it owns. `{body}` is where the exit goes; `{held}` is a
# boxed value the block is holding while it leaves.
BLOCKS = {
    "with-literal": "    with Src handled_by SrcHandler{{ get: || {held}.len() }} {{\n{body}\n    }}",
    "with-value": "    let h = SrcHandler{{ get: || {held}.len() }}\n"
    "    with Src handled_by h {{\n{body}\n    }}",
    "with-nested": "    with Src handled_by SrcHandler{{ get: || {held}.len() }} {{\n"
    "        with Src handled_by SrcHandler{{ get: || {held}.len() + 1 }} {{\n{body}\n        }}\n    }}",
    "region": "    region {{\n        let inside = [9, 9]\n        let _c = inside.len()\n{body}\n    }}",
    "plain-block": "    {{\n        let inside = {held}\n        let _c = inside.len()\n{body}\n    }}",
    "unsafe-block": "    unsafe {{\n        let inside = {held}\n        let _c = inside.len()\n{body}\n    }}",
    # A `parallel` owns a list of the sites that fired, built by the block and
    # freed when the tasks have run. Released by hand at its two ends, it was
    # kept by every early exit (F64); the block also had to learn to say it had
    # left, which F63 gave every block-shaped *expression* and this statement
    # needed of its own.
    "parallel": "    parallel {{\n        spawn {{\n            let _s = {held}.len()\n        }}\n{body}\n    }}",
    "spawn-body": "    parallel {{\n        spawn {{\n{body}\n        }}\n    }}",
    # No `spawn` at all: the list is built and freed with nothing to run, which
    # is the other end of the same release.
    "parallel-empty": "    parallel {{\n{body}\n    }}",
    # A literal under construction owns what it has built so far. An element
    # or field that *leaves* abandons it -- the value is held in a Rust local
    # and nothing releases it (F65). Four builders, one shape.
    "list-literal": "    let made = [1, {body_expr}]\n    let _n = made.len()",
    "map-literal": "    let made = {{ \"a\": {held}, \"b\": {body_expr_boxed} }}\n    let _n = made.len()",
    "tuple-literal": "    let made = ({held}, {body_expr_boxed})\n    let _n = made.0.len()",
    "struct-literal": "    let made = Pair{{ a: {held}, b: {body_expr_boxed} }}\n    let _n = made.a.len()",
    "if-branch": "    if {held}.len() > 0 {{\n{body}\n    }}",
    "match-arm": "    match {held}.len() {{\n        0 -> {{\n            let _z = 0\n        }}\n        _ -> {{\n{body}\n        }}\n    }}",
}

# How it is left. Each needs a different enclosing shape to be legal.
EXITS = {
    "fail": ("        fail(Bad)", "fallible"),
    "return": ("        return 1", "plain"),
    "break": ("        break", "loop"),
    "continue": ("        continue", "loop"),
    "falls-through": ("        let _k = 0", "plain"),
    "try": ("        let _t = try risky()", "fallible"),
}


# The exit as an expression, for the builders that need one in an element or
# a field rather than as a statement of its own.
EXIT_EXPRS = {
    "fail": ("if true {{ fail(Bad) }} else {{ {alt} }}", "fallible"),
    "return": ("if true {{ return 1 }} else {{ {alt} }}", "plain"),
    "break": ("if true {{ break }} else {{ {alt} }}", "loop"),
    "continue": ("if true {{ continue }} else {{ {alt} }}", "loop"),
    "falls-through": ("{alt}", "plain"),
    "try": ("if true {{ try risky_of({alt}) }} else {{ {alt} }}", "fallible"),
}

BUILDERS = ("list-literal", "map-literal", "tuple-literal", "struct-literal")


def build(bname, block, ename, exit_body, kind):
    held = "kept"
    # A block performs what its contents perform, and `unsafe { … }` performs
    # `unsafe`. Written without it the checker refuses every program in that
    # block and five quietly do not run -- the fault F58 found in its own
    # `spawn` context, in a different construct.
    if bname == "unsafe-block":
        row = "{io, unsafe, Src}"
    elif bname in ("parallel", "spawn-body", "parallel-empty"):
        row = "{io, async, Src}"
    else:
        row = "{Src}"
    if bname in BUILDERS:
        tpl, _ = EXIT_EXPRS[ename]
        inner = block.format(
            body_expr=tpl.format(alt="2"),
            body_expr_boxed=tpl.format(alt="[2]"),
            held=held,
        )
    else:
        inner = block.format(body=exit_body, held=held)
    if kind == "fallible":
        return (
            PRELUDE
            + "fn leaves(kept: [i64]) -> i64 ! Bad / " + row + " {\n"
            + inner
            + '\n    0\n}\n\nfn main() {\n    print("{leaves([1, 2]) ?? 0 - 1}")\n}\n'
        )
    if kind == "plain":
        return (
            PRELUDE
            + "fn leaves(kept: [i64]) -> i64 / " + row + " {\n"
            + inner
            + '\n    0\n}\n\nfn main() {\n    print("{leaves([1, 2])}")\n}\n'
        )
    # A loop, so `break` and `continue` have something to leave.
    return (
        PRELUDE
        + "fn leaves(kept: [i64]) -> i64 / " + row + " {\n"
        + "    var n = 0\n    for _i in 0..3 {\n"
        + "\n".join("    " + l for l in inner.splitlines())
        + "\n        n = n + 1\n    }\n    n\n}\n\n"
        'fn main() {\n    print("{leaves([1, 2])}")\n}\n'
    )


TRACE = re.compile(
    r"^(stack trace \(innermost first\):"
    r"|  [A-Za-z_][A-Za-z0-9_:]* at "
    r"|  \.\.\. [0-9]+ more frame\(s\))"
)


def strip_trace(text):
    return "\n".join(l for l in text.splitlines() if not TRACE.match(l))


def run(cmd, timeout=180):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="grasm_fuzz_exits_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, bad, declined, refused = 0, [], [], []
    try:
        for bname, block in BLOCKS.items():
            for ename, (exit_body, kind) in EXITS.items():
                src = build(bname, block, ename, exit_body, kind)
                with open(path, "w") as f:
                    f.write(src)
                label = f"{bname}/{ename}"
                rc, out, err = run([GRASM, "run", path])
                if rc != 0 and "error:" in err:
                    # The checker's own rules: a `region` may not `return` or
                    # `fail` through its brace ([T-REGION]), and a branch that
                    # must deliver a value has to end in an expression.
                    refused.append(label)
                    continue
                checked += 1
                bp = build_native(GRASM, path, exe, "--native")
                rc2, o2, err2 = bp.returncode, bp.stdout, bp.stderr
                if rc2 != 0:
                    first = (o2 + err2).strip().splitlines()
                    declined.append(f"{label}: {first[0] if first else '(silent)'}")
                    continue
                rc3, got, err3 = run([exe])
                if (out, rc, strip_trace(err)) != (got, rc3, strip_trace(err3)):
                    bad.append(label)
                    print(
                        f"DISAGREES {label}:\n  run:    exit {rc} {out.strip()!r}\n"
                        f"  native: exit {rc3} {got.strip()!r}\n{src}"
                    )
                    continue
                held = leaked(exe)
                if held:
                    bad.append(label)
                    print(f"LEAKS {label}: {held}\n{src}")
        for d in declined:
            print(f"  DECLINED (every shape here should compile): {d}")
        print(
            f"programs: {checked}   disagreed: {len(bad)}   "
            f"declined: {len(declined)}   refused by the checker: {len(refused)}"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad or declined else 0)


if __name__ == "__main__":
    main()
