#!/usr/bin/env python3
"""Every way a program fails, from every place it can fail.

A panic is a promise: the same message, the same exit status, whichever
engine ran the program. [E-PANIC-EXIT] fixes the status and [E-PANIC-TRACE]
says the trace after it is the one thing the engines may differ about.

What this crosses is *where* the fault happens. The same out-of-range index
takes a different route to the same panic from a closure, a `map` body, a
`spawn`, a handler operation, a `with` block and a fallible function -- each
has its own unwinding, its own scope releases, and its own idea of what to do
with a value that will never be produced. Every earlier crossing here asked
what a program computes; this one asks what it does when it cannot.

Half the faults are at the **boundary** -- `xs[len]`, `at(len)`,
`slice(1, len)` -- because that is where an off-by-one lives. Changing the
runtime's `>=` to `>` is invisible to `xs[5]` on a two-element list, which
trips a check that is wrong by one just as surely as a correct one; it is
`xs[2]` that catches it, and it catches it in every context at once.

    scripts/fuzz_panics.py [--keep]
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


_fresh = _lib("fresh")
_fresh.fresh_or_stop(GRASM)


PRELUDE = """type E = { why: str }

effect Src {
    fn get() -> i64
}
"""

# How the program fails. The boundary cases are the point; the far-out-of-range
# ones are here to show that they are not enough on their own.
FAULTS = {
    "index-far": 'let xs = [1, 2]\n    let i = 5\n    print("{xs[i]}")',
    "index-len": 'let xs = [1, 2]\n    let i = 2\n    print("{xs[i]}")',
    "index-neg": 'let xs = [1, 2]\n    let i = 0 - 1\n    print("{xs[i]}")',
    "bytes-at-far": 'let b = bytes_of([1])\n    print("{b.at(9)}")',
    "bytes-at-len": 'let b = bytes_of([1, 2])\n    let i = 2\n    print("{b.at(i)}")',
    "slice-far": 'let b = bytes_of([1])\n    print("{b.slice(0, 9)}")',
    "slice-edge": 'let b = bytes_of([1, 2])\n    print("{b.slice(1, 2)}")',
    "remove-far": 'var xs = [1]\n    print("{xs.remove(9)}")',
    "remove-len": 'var xs = [1, 2]\n    print("{xs.remove(2)}")',
    "div-zero": 'let z = 0\n    print("{10 / z}")',
    "mod-zero": 'let z = 0\n    print("{10 % z}")',
    "expect": 'let o: Option[i64] = None\n    print("{o.expect("gone")}")',
    "panic": 'panic("stated")',
}

# Where it happens. Each has its own unwinding.
CONTEXTS = [
    "top",
    "in-fn",
    "in-closure",
    "in-map",
    "in-spawn",
    "in-handler",
    "in-with",
    "in-fallible",
]

TEMPLATES = {
    "in-closure": "    let go = || {{\n{f}\n    }}\n    go()",
    "in-map": '    let out = [1].map(|_x| {{\n{f}\n        0\n    }})\n    print("{{out}}")',
    "in-spawn": "    parallel {{\n        spawn {{\n{f}\n        }}\n    }}",
    "in-handler": '    with Src handled_by SrcHandler{{ get: || {{\n{f}\n        0\n    }} }} {{\n'
    '        print("{{Src.get()}}")\n    }}',
    "in-with": '    with Src handled_by SrcHandler{{ get: || 1 }} {{\n{f}\n    }}',
}


def build(fault, ctx):
    if ctx == "top":
        return PRELUDE + "fn main() / {io} {\n    " + fault + "\n}\n"
    if ctx == "in-fn":
        return (
            PRELUDE
            + "fn deep() / {io} {\n    "
            + fault
            + "\n}\n\nfn mid() / {io} {\n    deep()\n}\n\n"
            "fn main() / {io} {\n    mid()\n}\n"
        )
    if ctx == "in-fallible":
        return (
            PRELUDE
            + "fn risky() -> i64 ! E / {io} {\n    "
            + fault
            + '\n    1\n}\n\nfn main() / {io} {\n    print("{risky() ?? 0 - 1}")\n}\n'
        )
    body = "    " + fault.replace("\n    ", "\n        ")
    # `parallel`/`spawn` perform `async`, so `main` has to declare it. Written
    # without, the checker refuses every one of them and the whole context
    # quietly does not run -- which is how this axis was skipped in the probe
    # this file came from, silently, for all thirteen faults.
    row = "{io, async}" if ctx == "in-spawn" else "{io}"
    return PRELUDE + "fn main() / " + row + " {\n" + TEMPLATES[ctx].format(f=body) + "\n}\n"


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

    work = tempfile.mkdtemp(prefix="grasm_fuzz_panics_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, bad, declined, rejected = 0, [], [], []
    try:
        for fname, fault in FAULTS.items():
            for ctx in CONTEXTS:
                src = build(fault, ctx)
                with open(path, "w") as f:
                    f.write(src)
                label = f"{fname}/{ctx}"
                rc, out, err = run([GRASM, "run", path])
                # The checker refuses some pairings -- a `panic` in a context
                # that must produce a value, say. That is its decision.
                if rc != 0 and not out and "error:" in err:
                    rejected.append(label)
                    continue
                checked += 1
                rc2, o2, err2 = run([GRASM, "build", path, "--native", "-o", exe])
                if rc2 != 0:
                    first = (o2 + err2).strip().splitlines()
                    declined.append(f"{label}: {first[0] if first else '(silent)'}")
                    continue
                rc3, got, err3 = run([exe])
                if (out, rc, strip_trace(err)) != (got, rc3, strip_trace(err3)):
                    bad.append(label)
                    print(
                        f"DISAGREES {label}:\n"
                        f"  run:    exit {rc} {out.strip()!r} {strip_trace(err).strip()!r}\n"
                        f"  native: exit {rc3} {got.strip()!r} {strip_trace(err3).strip()!r}"
                    )
        for d in declined:
            print(f"  DECLINED (every shape here should compile): {d}")
        print(
            f"programs: {checked}   disagreed: {len(bad)}   "
            f"declined: {len(declined)}   not a panic: {len(rejected)}"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad or declined else 0)


if __name__ == "__main__":
    main()
