#!/usr/bin/env python3
"""What a closure captures, once per representation it can be.

A capture does not travel in a register. It is written into an environment --
eight bytes per capture, a pointer for anything boxed and the scalar's own
bits for anything else -- and read back out when the closure runs. So the
*representation* of a captured value decides which code writes it and which
reads it, and either half can be missing a case.

F50 is what that looks like: `value_to_bits` and `bits_to_value` both listed
`I64 | F64 | Bool | Ref` and fell through for the rest. Right for a boxed
value, which is already a word; wrong for a unit, whose Cranelift type is
`I8`. A handler operation capturing `()` stored an `I8` and read back an
`I64`, and Cranelift's own frontend rejected the function -- a compiler
panic, not a refusal.

It had been reachable for as long as handlers had been, and none of the
crossings before it found anything, because they all built their values from
**literals**. A closure returning a literal never reads its environment. The
axis is not the value's type alone; it is the value's type *and* whether it
arrived through an environment.

The shapes below are the ways an environment gets read: once per element
through `map`, twice through `fold`, after the scope that owned the value has
gone, out of a list of function values, through a parameter, and from several
threads at once. The last is where a missing reference count shows: dropping
the borrow taken for a parallel capture leaves every boxed representation
crashing with an empty stdout, and nothing else in this file notices.

    scripts/fuzz_captures.py [--keep]
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

_spec = _ilu.spec_from_file_location(
    "grasm_fresh",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "fresh.py"),
)
_fresh = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_fresh)
_fresh.fresh_or_stop(GRASM)


PRELUDE = """type P = { n: i64 }
type B[T] = { v: T }
type Col = | Red | Green
"""

# Every representation a value can have, as (type, literal). `unit` is the one
# that crashed; the boxed ones are the ones a missing reference count kills.
VALUES = [
    ("i64", "i64", "1"),
    ("f64", "f64", "1.5"),
    ("bool", "bool", "true"),
    ("str", "str", '"a"'),
    ("unit", "()", "()"),
    ("bytes", "bytes", "bytes_of([1])"),
    ("list", "[i64]", "[1, 2]"),
    ("tuple", "(i64, str)", '(1, "a")'),
    ("option", "Option[i64]", "Some(1)"),
    ("struct", "P", "P{ n: 1 }"),
    ("generic", "B[i64]", "B{ v: 1 }"),
    ("sum", "Col", "Red"),
]


def shapes(ty, lit):
    """One program per way an environment is read."""
    return {
        # Read once per element.
        "map": (
            f"fn main() {{\n    let held = {lit}\n"
            f"    let out = [1, 2, 3].map(|_x| held)\n"
            f'    print("{{out}}")\n}}\n'
        ),
        # Read as the seed and again per element.
        "fold": (
            f"fn main() {{\n    let held = {lit}\n"
            f"    let out = [1, 2].fold(held, |_a, _x| held)\n"
            f'    print("{{out}}")\n}}\n'
        ),
        # Read after the scope that owned the value is gone.
        "escaping": (
            f"fn make() -> fn() -> {ty} {{\n    let held = {lit}\n    || held\n}}\n"
            f"fn main() {{\n    let f = make()\n"
            f'    print("{{f()}} {{f()}}")\n}}\n'
        ),
        # Two closures over one capture, reached through a list.
        "in-a-list": (
            f"fn main() {{\n    let held = {lit}\n"
            f"    let fs: [fn() -> {ty}] = [|| held, || held]\n"
            f'    print("{{fs[0]()}} {{fs[1]()}}")\n}}\n'
        ),
        # Called from a function that never saw the literal.
        "passed": (
            f"fn call(f: fn() -> {ty}) -> {ty} {{\n    f()\n}}\n"
            f"fn main() {{\n    let held = {lit}\n"
            f'    print("{{call(|| held)}}")\n}}\n'
        ),
        # Read from several threads. The borrow taken here is the traffic that
        # made the reference count have to be atomic.
        "par": (
            f"fn main() {{\n    let held = {lit}\n"
            f"    let out = [1, 2, 3].par_map(|_x| held)\n"
            f'    print("{{out.len()}}")\n}}\n'
        ),
        # A `spawn` body sees the enclosing scope and does not capture like a
        # closure ([E-SPAWN-SHARED]), so it reaches a value by a third route
        # again -- and it reaches it from another thread, where a reference
        # count is the only thing keeping the value alive.
        "spawn-reads": (
            f"fn main() {{\n    let held = {lit}\n    var log = \"\"\n"
            f"    parallel {{\n        spawn {{\n"
            f"            log += \"{{held}}\"\n        }}\n    }}\n"
            f'    print(log)\n}}\n'
        ),
        # Two of them, so the value is read from two threads at once.
        "two-spawns": (
            f"fn main() {{\n    let held = {lit}\n    var log = \"\"\n"
            f"    parallel {{\n        spawn {{\n"
            f"            log += \"{{held}}\"\n        }}\n"
            f"        spawn {{\n            log += \"{{held}}\"\n        }}\n    }}\n"
            f'    print(log)\n}}\n'
        ),
        # Built on a worker and printed there: its allocation is made on one
        # thread and released on another, which is what `ESCAPED` is for.
        "built-in-spawn": (
            f"fn main() {{\n    var log = \"\"\n"
            f"    parallel {{\n        spawn {{\n            let made = {lit}\n"
            f"            log += \"{{made}}\"\n        }}\n    }}\n"
            f'    print(log)\n}}\n'
        ),
        # An effect handler's operation is a closure too, and it is the one
        # whose environment F50 was in.
        "handler": (
            f"effect Src {{\n    fn get() -> {ty}\n}}\n"
            f"fn ask() -> {ty} / {{Src}} {{\n    Src.get()\n}}\n"
            f"fn main() {{\n    let held = {lit}\n"
            f"    with Src handled_by SrcHandler{{ get: || held }} {{\n"
            f'        print("{{ask()}}")\n    }}\n}}\n'
        ),
        # A closure inside a closure: the value is copied out of one
        # environment and into another. Every nesting was refused until F51,
        # because typing the outer one means typing its body and its body
        # calls the inner one.
        "nested": (
            f"fn main() {{\n    let held = {lit}\n"
            f"    let outer = || {{\n        let inner = || held\n        inner()\n    }}\n"
            f'    print("{{outer()}}")\n}}\n'
        ),
        # A closure returning a closure, so the capture outlives two scopes.
        "returned-twice": (
            f"fn make() -> fn() -> fn() -> {ty} {{\n    let held = {lit}\n"
            f"    || || held\n}}\n"
            f"fn main() {{\n    let f = make()\n    let g = f()\n"
            f'    print("{{g()}} {{g()}}")\n}}\n'
        ),
        # A `var` written after the closure is made. A closure captures values
        # at the moment it is created, so the answer must not move.
        "var-changed-after": (
            f"fn main() {{\n    var held = {lit}\n"
            f"    let f = || held\n    held = {lit}\n"
            f'    print("{{f()}}")\n}}\n'
        ),
        # A handler installed inside a closure, so the operation's
        # environment is built while another environment is being read.
        "handler-in-closure": (
            f"effect Src {{\n    fn get() -> {ty}\n}}\n"
            f"fn main() {{\n    let held = {lit}\n"
            f"    let outer = || {{\n"
            f"        with Src handled_by SrcHandler{{ get: || held }} {{\n"
            f"            Src.get()\n        }}\n    }}\n"
            f'    print("{{outer()}}")\n}}\n'
        ),
        # Three environments deep: a closure inside a handler operation
        # inside a closure.
        "closure-in-handler-in-closure": (
            f"effect Src {{\n    fn get() -> {ty}\n}}\n"
            f"fn main() {{\n    let held = {lit}\n"
            f"    let outer = || {{\n"
            f"        with Src handled_by SrcHandler{{ get: || {{\n"
            f"            let inner = || held\n            inner()\n        }} }} {{\n"
            f"            Src.get()\n        }}\n    }}\n"
            f'    print("{{outer()}}")\n}}\n'
        ),
        # A generic function returning a closure over its own parameter: the
        # capture's representation is not known until the instantiation is.
        "generic-returns-closure": (
            f"fn hold[T](x: T) -> fn() -> T {{\n    || x\n}}\n"
            f"fn main() {{\n    let f = hold({lit})\n"
            f'    print("{{f()}} {{f()}}")\n}}\n'
        ),
        # A closure declared inside a generic function.
        "closure-in-generic": (
            f"fn use1[T](x: T) -> T {{\n    let inner = || x\n    inner()\n}}\n"
            f"fn main() {{\n"
            f'    print("{{use1({lit})}}")\n}}\n'
        ),
        # The capture reached through a generic struct's field.
        "capture-through-generic": (
            f"fn main() {{\n    let held = B{{ v: {lit} }}\n"
            f"    let f = || held.v\n"
            f'    print("{{f()}}")\n}}\n'
        ),
        # A generic function whose closure goes to `map`, so the element type
        # and the capture type are the same parameter.
        "generic-map-capture": (
            f"fn each[T](x: T) -> [T] {{\n    [1, 2].map(|_i| x)\n}}\n"
            f"fn main() {{\n"
            f'    print("{{each({lit})}}")\n}}\n'
        ),
        # The same, with the handler itself a value the block never saw
        # written -- a different path through the backend.
        "handler-as-value": (
            f"effect Src {{\n    fn get() -> {ty}\n}}\n"
            f"fn install(h: SrcHandler) / {{io, Src}} {{\n"
            f"    with Src handled_by h {{\n"
            f'        print("{{Src.get()}}")\n    }}\n}}\n'
            f"fn main() {{\n    let held = {lit}\n"
            f"    install(SrcHandler{{ get: || held }})\n}}\n"
        ),
    }


# [E-PANIC-TRACE]: the trace block is the one thing the engines may differ
# about. The same rule as scripts/lib/trace.sh.
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

    work = tempfile.mkdtemp(prefix="grasm_fuzz_captures_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, bad, declined = 0, [], []
    try:
        for name, ty, lit in VALUES:
            for sname, body in shapes(ty, lit).items():
                src = PRELUDE + body
                with open(path, "w") as f:
                    f.write(src)
                checked += 1
                label = f"{name}/{sname}"
                rc, out, err = run([GRASM, "run", path])
                if rc != 0:
                    # The generator emits well-typed programs, so this is a
                    # fault in the generator or in the interpreter.
                    bad.append(label)
                    print(f"REJECTED {label}: {strip_trace(err).strip()}\n{src}")
                    continue
                bp = build_native(GRASM, path, exe, "--native")
                rc2, o2, err2 = bp.returncode, bp.stdout, bp.stderr
                if rc2 != 0:
                    first = (o2 + err2).strip().splitlines()
                    first = first[0] if first else "(silent)"
                    # A refusal is honest; a panic or a verifier failure is the
                    # backend failing against itself, which is what F50 was.
                    if "codegen bug" in first or "panicked" in (o2 + err2):
                        bad.append(label)
                        print(f"COMPILER FAULT {label}: {first}\n{src}")
                    else:
                        declined.append(f"{label}: {first.split(': ', 1)[-1]}")
                    continue
                rc3, got, err3 = run([exe])
                if (out, rc, strip_trace(err)) != (got, rc3, strip_trace(err3)):
                    bad.append(label)
                    print(
                        f"DISAGREES {label}: run={(out.strip(), rc)} "
                        f"native={(got.strip(), rc3)}"
                    )
                    continue
                held = leaked(exe)
                if held:
                    bad.append(label)
                    print(f"LEAKS {label}: {held}")
        # Every shape here is one the backend compiles, so unlike the other
        # generators a decline is a failure rather than an honest refusal.
        # Removing the F51 fix made this report `declined: 12` and exit 0 --
        # twelve programs of coverage lost while the runner printed PASS,
        # which is the same shape as reading a gate's summary instead of its
        # status. `nativegaps.sh` caught that one; this should not have needed
        # it to.
        for d in declined:
            print(f"  DECLINED (every shape here should compile): {d}")
        print(
            f"programs: {checked}   disagreed: {len(bad)}   "
            f"declined: {len(declined)}"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad or declined else 0)


if __name__ == "__main__":
    main()
