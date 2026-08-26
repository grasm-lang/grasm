#!/usr/bin/env python3
"""Every spelling by which a C function can be reached, in both directions.

Naming a C function is the permission to call it, so [T-EXTERN-VALUE] charges a
bare mention what a call costs, and [T-UNSAFE-UFCS] says the method spelling is
the same call as the free one. Two rounds found holes here that no other gate
covers: `n.labs()` charged nothing while `labs(n)` charged `io`, so a package
declaring `capabilities = ["unsafe"]` was reported as reaching no `io` while
its binary called libc; and `unsafe { sqrt }` carried one onto `par_map`'s
workers, which [T-PAR-MAP-PURE] calls a compile error rather than a race.

Each spelling is checked twice, and the oracle is exact both ways:

  * declared `/ {unsafe}` — the `io` a C function costs is missing, so
    `grasm check` must **fail**;
  * declared `/ {io, unsafe}` — everything performed is declared, so it must
    **succeed**, and the program must run.

A spelling that passes the first and fails the second is a hole; one that
fails the second is a false positive. Both print the whole program, since the
grid position alone does not say what to look at.

A third round covers the *pointer* boundary ([T-EXTERN-POINTER]). A `str` or
`bytes` argument crosses as a pointer, and the conversion lives at the call
site — so every spelling that reaches a C function some other way is a place it
could be missed, and missing it hands C a boxed value's address instead of the
text. The oracle is exact: `strlen(s)` must equal `s.byte_len()`, for strings on
both sides of the small-string boundary and on both sides of ASCII. A backend
that declines a spelling is reported and not failed; one that answers *wrongly*
is the defect this round exists for.

    scripts/fuzz_extern.py [--keep]
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

_spec = _ilu.spec_from_file_location(
    "grasm_fresh", os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "fresh.py")
)
_fresh = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_fresh)
_fresh.fresh_or_stop(GRASM)


DECL = "extern fn labs(x: i64) -> i64\n\n"

# How the C function is reached, written inside an `unsafe` block whose value
# is an `i64`. Each is the same call, so each costs the same row.
SPELLINGS = [
    ("direct", "labs(n)"),
    ("method", "n.labs()"),
    ("pipeline", "n |> labs"),
    ("let-then-call", "let f = labs\n        f(n)"),
    ("block-then-call", "let f = { labs }\n        f(n)"),
    ("nested-block", "let f = { { labs } }\n        f(n)"),
    ("map", "[n].map(labs)[0]"),
    ("map-through-let", "let f = labs\n        [n].map(f)[0]"),
    ("list-element", "let fs = [labs]\n        fs[0](n)"),
    ("tuple-component", "let t = (labs, 1)\n        t.0(n)"),
    ("option-payload", "let o = Some(labs)\n        (o ?? labs)(n)"),
    ("closure-body", "let f = |x| labs(x)\n        f(n)"),
    ("closure-value", "let f = |x| x\n        [n].map(labs)[0] + f(0)"),
    ("fold", "[n].fold(0, |a, b| a + labs(b))"),
    ("if-branch", "if n > 0 { labs(n) } else { labs(0 - n) }"),
    ("match-arm", "match n > 0 {\n            true -> labs(n)\n            false -> 0\n        }"),
    ("interpolation", "let s = \"{labs(n)}\"\n        s.len()"),
    ("helper-arg", "call(labs, n)"),
]

# The helper's parameter declares the row it will be handed. A written type
# is the contract ([T-FN-TYPE-ROW-CARRIED]), so a `fn(i64) -> i64` there
# would rightly refuse a C function and the probe would measure the
# generator rather than the compiler.
HELPER = (
    "fn call(f: fn(i64) -> i64 / {io, unsafe}, n: i64) -> i64 / {io, unsafe} {\n"
    "    f(n)\n}\n\n"
)

# The row the enclosing function declares. `unsafe` alone is missing the `io` a
# C function costs; the pair is complete.
ROWS = [("{unsafe}", False), ("{io, unsafe}", True)]


def build(spelling, row):
    return (
        DECL
        + HELPER
        + f"fn work(n: i64) -> i64 / {row} {{\n    unsafe {{\n        {spelling}\n    }}\n}}\n"
        + "\nfn main() {\n    print(\"{work(0 - 5)}\")\n}\n"
    )


# --- the pointer boundary -------------------------------------------------
#
# Only the spellings the native backend supports at all: it declines indirect
# calls generally, which is a refusal rather than a wrong answer and is not
# what this round is looking for.
POINTER_SPELLINGS = [
    ("direct", "strlen(s)"),
    ("method", "s.strlen()"),
    ("pipeline", "s |> strlen"),
    ("map", "[s].map(strlen)[0]"),
    ("if-branch", "if s.len() > 0 { strlen(s) } else { strlen(s) }"),
    ("interpolation", "\"{strlen(s)}\".parse_int() ?? 0 - 1"),
    ("helper-arg", "measure(strlen, s)"),
]

# Both sides of the fifteen-byte inline boundary, both sides of ASCII, and the
# empty string — the places a length is most likely to be wrong.
SUBJECTS = [
    '""',
    '"a"',
    '"hello"',
    '"0123456789abcde"',
    '"0123456789abcdef"',
    '"0123456789abcdefghij"',
    r'"\u{03b1}\u{03b2}\u{03b3}"',
    r'"caf\u{e9} na\u{ef}ve"',
]

PTR_DECL = "extern fn strlen(s: str) -> i64\n\n"
PTR_HELPER = (
    "fn measure(f: fn(str) -> i64 / {io, unsafe}, s: str) -> i64 / {io, unsafe} {\n"
    "    f(s)\n}\n\n"
)


def build_pointer(spelling):
    """A program printing, for each subject, the C answer and the Grasm one."""
    subjects = ", ".join(SUBJECTS)
    return (
        PTR_DECL
        + PTR_HELPER
        + f"fn measured(s: str) -> i64 / {{io, unsafe}} {{\n"
        + f"    unsafe {{\n        {spelling}\n    }}\n}}\n\n"
        + "fn main() / {io, unsafe} {\n"
        + f"    let subjects = [{subjects}]\n"
        + "    var out = \"\"\n"
        + "    for s in subjects {\n"
        + "        out = out + \"{measured(s)}/{s.byte_len()} \"\n"
        + "    }\n"
        + "    print(out.trim())\n}\n"
    )


def run(cmd):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    return p.returncode, p.stdout, p.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="grasm_fuzz_extern_")
    path = os.path.join(work, "p.gr")
    leaks, false_positives, checked = [], [], 0
    try:
        for name, spelling in SPELLINGS:
            for row, should_pass in ROWS:
                src = build(spelling, row)
                with open(path, "w") as f:
                    f.write(src)
                rc, out, err = run([GRASM, "check", path])
                checked += 1
                text = (out + err).strip()
                if should_pass and rc != 0:
                    false_positives.append(name)
                    first = text.splitlines()[0] if text else ""
                    print(f"FALSE POSITIVE {name}: {first}\n{src}")
                elif not should_pass and rc == 0:
                    leaks.append(name)
                    print(f"LEAK {name}: a C function reached `/ {row}`\n{src}")
                # A program that passes must also run.
                if should_pass and rc == 0:
                    rc2, _, err2 = run([GRASM, "run", path])
                    if rc2 != 0:
                        false_positives.append(f"{name}/run")
                        print(f"RUN FAILED {name}: {err2.strip()}\n{src}")
        # Round three: the pointer boundary, on both engines.
        declined = []
        for name, spelling in POINTER_SPELLINGS:
            src = build_pointer(spelling)
            with open(path, "w") as f:
                f.write(src)
            checked += 1
            rc, out, err = run([GRASM, "run", path])
            if rc != 0:
                false_positives.append(f"pointer/{name}")
                print(f"POINTER REJECTED {name}: {(out + err).strip()}\n{src}")
                continue
            interp = out.strip()
            wrong = [p for p in interp.split() if "/" in p and p.split("/")[0] != p.split("/")[1]]
            if wrong:
                false_positives.append(f"pointer/{name}")
                print(f"POINTER WRONG {name}: C answered {wrong} (c/grasm)\n{src}")
                continue
            exe = os.path.join(work, "p.bin")
            rc2, _, err2 = run([GRASM, "build", path, "--native", "-o", exe])
            if rc2 != 0:
                # A refusal is honest; a wrong answer is not.
                declined.append(name)
                continue
            rc3, out3, err3 = run([exe])
            if rc3 != 0 or out3.strip() != interp:
                false_positives.append(f"pointer/{name}/native")
                print(
                    f"POINTER DISAGREES {name}:\n  interpreter: {interp}\n"
                    f"  native:      {out3.strip()} {err3.strip()}\n{src}"
                )
                continue
            # The runtime's own allocations, not the C library's: a string
            # handed to a C function is boxed on this side of the boundary and
            # has to come back (F57).
            held = leaked(exe)
            if held:
                false_positives.append(f"pointer/{name}/leak")
                print(f"POINTER LEAKS {name}: {held}\n{src}")
        if declined:
            print(f"  native declined (not a defect): {', '.join(declined)}")
        print(
            f"programs: {checked}   leaked: {len(leaks)}   "
            f"false positives: {len(false_positives)}"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if leaks or false_positives else 0)


if __name__ == "__main__":
    main()
