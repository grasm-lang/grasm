#!/usr/bin/env python3
"""Every builtin method, on every shape of receiver, on both engines.

F17 is the reason this exists. `byte_len` was added to the native backend's
method table and *not* to the table beside it that says what each method
returns, whose last arm was a catch-all. The compiled program called `.len()`
on the integer 10 as though it were a pointer and took SIGSEGV, while
`grasm run` printed the right answer.

The unit test written alongside the feature passed, because a **literal**
receiver takes a different path in the backend. It took a real program with a
loop variable to expose it. So the grid here is method × *receiver shape*: the
same call written six ways, which is the axis that hid the defect.

One program per (type, shape), printing every method of that type on its own
line, compared against the interpreter. A native build that declines is
reported and not failed; one that answers differently is the defect.

    scripts/fuzz_methods.py [--keep]
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

_spec = _ilu.spec_from_file_location(
    "grasm_fresh",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "fresh.py"),
)
_fresh = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_fresh)
_fresh.fresh_or_stop(GRASM)


# (type, a literal of that type, [method call written against `R`])
GROUPS = [
    (
        "str",
        '"Hello, World"',
        [
            "R.len()", "R.byte_len()", "R.is_empty()", "R.substr(1, 3)",
            "R.chars().len()", "R.lines().len()", 'R.split(",").len()',
            "R.words().len()", 'R.contains("World")', 'R.starts_with("He")',
            'R.ends_with("ld")', 'R.index_of("W") ?? 0 - 1', 'R.replace("l", "L")',
            "R.trim()", "R.trim_start()", "R.trim_end()", "R.to_upper()",
            "R.to_lower()", "R.repeat(2).len()", 'R.pad_start(20, "-")',
            'R.pad_end(20, "-")', "R.parse_int() ?? 0 - 1",
            "R.parse_int_base(16) ?? 0 - 1", "R.parse_float() ?? 0.0 - 1.0",
            "R.code_at(0)", "R.codes().len()", "R.to_bytes().len()",
        ],
    ),
    (
        "i64",
        "0 - 42",
        [
            "R.abs()", "R.as_f64()", "R.to_str()", "R.min(7)", "R.max(7)",
            "R.cmp(7)", "R.to_base(16)", "R.to_base(2)", "R.bit_and(12)",
            "R.bit_or(12)", "R.bit_xor(12)", "R.bit_not()", "R.shl(2)",
            "R.shr(2)", "R.ushr(2)", "R.abs().to_char()",
        ],
    ),
    (
        "f64",
        "2.25",
        [
            "R.abs()", "R.as_i64()", "R.to_str()", "R.sqrt()", "R.floor()",
            "R.ceil()", "R.round()", "R.is_nan()", "R.is_finite()",
            "R.to_fixed(3)", "R.ln()", "R.log10()", "R.log2()", "R.exp()",
            "R.sin()", "R.cos()", "R.tan()", "R.hypot(1.0)", "R.atan2(1.0)",
            "R.to_degrees()", "R.to_radians()", "R.cbrt()", "R.log(2.0)",
            "R.sinh()", "R.cosh()", "R.tanh()",
            "(R / 4.0).asin()", "(R / 4.0).acos()", "R.atan()",
        ],
    ),
    # A map and an `Option` carry the same representations a list does, and the
    # same question applies: an operation that works on one and not another.
    # The container itself is interpolated — `m`, `m.values()`, `o` — because a
    # count cannot show that a unit came back as `0`.
    (
        "map of unit",
        "built_unit()",
        [
            "R.len()", "R.is_empty()", 'R.has("a")', 'R.get("a")',
            'R.get("nope")', "R.keys()", "R.values()", "R.entries()", "R",
        ],
    ),
    (
        "map of tuple",
        "built_tuple()",
        [
            "R.len()", 'R.has("a")', 'R.get("a")', "R.keys()", "R.values()",
            "R.entries()", "R",
        ],
    ),
    (
        "option of unit",
        "Some(())",
        ["R.is_some()", "R.is_none()", "R", "R.unwrap_or(())"],
    ),
    (
        "option of tuple",
        'Some((1, "a"))',
        ["R.is_some()", "R.is_none()", "R", 'R.unwrap_or((0, "z"))'],
    ),
    (
        "option of list",
        "Some([1, 2])",
        ["R.is_some()", "R.is_none()", "R", "R.unwrap_or([0])"],
    ),
    # The same list surface, once per *element representation* the runtime
    # distinguishes. A list of `i64` is stored inline and a list of tuples is
    # boxed, and an operation that works on one and not the other is how
    # `[(), ()]` came to print as `[0, 0]` (F37) and how a `write_file`'s unit
    # came back from a journal as an integer. Only the methods that do not care
    # what the element *is* — the rest are covered by the `list` group below.
    (
        "list of unit",
        "[(), (), ()]",
        [
            "R.len()", "R.is_empty()", "R", "R.rev()", "R.slice(0, 2)",
            "R.map(|v| v)", "R.first()", "R.last()", "R.get(0)",
        ],
    ),
    (
        "list of tuple",
        '[(1, "a"), (2, "b")]',
        [
            "R.len()", "R.is_empty()", "R", "R.rev()", "R.slice(0, 1)",
            "R.map(|v| v)", "R.first()", "R.last()", "R.get(0)",
        ],
    ),
    (
        "list of option",
        "[Some(1), None, Some(3)]",
        [
            "R.len()", "R.is_empty()", "R", "R.rev()", "R.slice(0, 2)",
            "R.map(|v| v)", "R.first()", "R.last()", "R.get(0)",
        ],
    ),
    (
        "list of bool",
        "[true, false, true]",
        [
            "R.len()", "R.is_empty()", "R", "R.rev()", "R.slice(0, 2)",
            "R.contains(true)", "R.any(|v| v)", "R.all(|v| v)",
            "R.map(|v| v)", "R.first()", "R.get(0)",
        ],
    ),
    (
        "list of bytes",
        "[bytes_of([1]), bytes_of([2])]",
        [
            "R.len()", "R.is_empty()", "R", "R.rev()", "R.slice(0, 1)",
            "R.map(|v| v)", "R.first()", "R.get(0)",
        ],
    ),
    (
        "list",
        "[3, 1, 2]",
        [
            "R.len()", "R.is_empty()", "R.get(0) ?? 0 - 1", "R.first() ?? 0 - 1",
            "R.last() ?? 0 - 1", "R.rev().len()", "R.slice(0, 2).len()",
            "R.contains(3)", "R.sum()", "R.min() ?? 0 - 1", "R.max() ?? 0 - 1",
            "R.fold(0, |a, b| a + b)", "R.map(|v| v * 2).len()",
            "R.filter(|v| v > 1).len()", "R.any(|v| v > 2)", "R.all(|v| v > 0)",
            "R.find(|v| v > 1) ?? 0 - 1", "R.enumerate().len()",
            "R.zip([1, 2]).len()", "R.par_map(|v| v + 1).len()",
            "R.par_filter(|v| v > 1).len()",
            "R.map(|v| v.to_str()).join(\"-\")",
            "R.enumerate().len()",
        ],
    ),
    (
        "map",
        "built()",
        [
            "R.len()", "R.is_empty()", 'R.get("a") ?? 0 - 1', 'R.has("a")',
            "R.keys().len()", "R.values().len()", "R.entries().len()",
            'R.keys().join(",")',
        ],
    ),
    (
        "option",
        "Some(7)",
        [
            "R.is_some()", "R.is_none()", "R.unwrap_or(0 - 1)",
            'R.expect("must be there")', "R ?? 0 - 1",
        ],
    ),
    (
        "bytes",
        "bytes_of([1, 2, 255])",
        [
            "R.len()", "R.is_empty()", "R.at(2)", "R.slice(0, 2).len()",
            "R.concat(bytes_of([9])).len()", "R.to_list().len()",
            "R.to_str().len()",
        ],
    ),
    # A `str` and a `bytes` have no element type to vary, but they do have
    # *contents*, and that is the axis: an operation that works on ASCII and
    # not on a character outside it. `len` counts characters and `byte_len`
    # counts bytes, so anything multibyte is where the two disagree if they
    # are going to. The interpreter holds a Rust `String` and the compiled
    # engines hold the runtime's, which is two implementations of the same
    # promise.
    (
        "str empty",
        '""',
        [
            "R.len()", "R.byte_len()", "R.is_empty()", "R.chars().len()",
            "R.codes().len()", "R.lines().len()", 'R.split(",").len()',
            "R.words().len()", "R.trim()", "R.to_upper()", "R.to_bytes().len()",
            'R.contains("a")', 'R.pad_start(3, "-")', "R.repeat(2)", "R",
        ],
    ),
    (
        "str multibyte",
        '"h\u00e9\u65e5\U0001d11ez"',
        [
            "R.len()", "R.byte_len()", "R.chars().len()", "R.codes().len()",
            "R.code_at(0)", "R.code_at(2)", "R.substr(0, 2)", "R.to_upper()",
            "R.to_lower()", "R.to_bytes().len()", "R.repeat(2).len()",
            'R.contains("\u65e5")', 'R.index_of("z") ?? 0 - 1',
            'R.replace("\u65e5", "X")', "R.trim()", "R",
        ],
    ),
    (
        "str whitespace",
        '"  a\\tb\\nc  "',
        [
            "R.len()", "R.byte_len()", "R.lines().len()", "R.words().len()",
            "R.trim()", "R.trim_start()", "R.trim_end()", 'R.split(" ").len()',
            "R.to_bytes().len()", "R",
        ],
    ),
    (
        "bytes invalid utf8",
        "bytes_of([255, 254, 65])",
        [
            "R.len()", "R.is_empty()", "R.at(0)", "R.to_list().len()",
            "R.to_str().byte_len()", "R.slice(1, 2).len()",
            "R.concat(bytes_of([0])).len()", "R",
        ],
    ),
    # The empty literals. They have nothing to read a type off and take one
    # from the annotation the checker resolved, and the six positions below
    # reach that answer by six different routes -- one of which is a list
    # literal's element, where an empty `map()` was refused (F52).
    (
        "empty map",
        "map()",
        [
            "R.len()", "R.is_empty()", 'R.has("a")', 'R.get("a")',
            "R.keys().len()", "R.values().len()", "R.entries().len()", "R",
        ],
    ),
    (
        "empty list",
        "[]",
        [
            "R.len()", "R.is_empty()", "R.first()", "R.last()",
            "R.rev().len()", 'R.map(|v| v + 1).len()', "R",
        ],
    ),
    (
        "bytes with nul",
        "bytes_of([0, 65, 0])",
        [
            "R.len()", "R.at(0)", "R.at(1)", "R.to_list().len()",
            "R.to_str().byte_len()", "R.slice(0, 2).len()", "R",
        ],
    ),
]

# How the receiver is spelled. `{lit}` is the literal, `{calls}` the printing
# body written against `R`.
SHAPES = {
    "literal": "    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    "local": "    let R = {lit}\n    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    "var-local": "    var R = {lit}\n    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    # The shape that exposed F17: nothing in the backend's fast paths sees a
    # literal here.
    "loop-var": "    for R in [{lit}] {{\n        let out = [\n{calls}\n        ]\n        print(out.join(\"|\"))\n    }}\n",
    "field": "    let holder = Box{{ v: {lit} }}\n    let R = holder.v\n    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    "element": "    let holder = [{lit}]\n    let R = holder[0]\n    let out = [\n{calls}\n    ]\n    print(out.join(\"|\"))\n",
    # The *call* in a probed position, rather than the receiver. A list
    # literal's element is the one place the backend has to name a method's
    # result type before it evaluates anything, and every arm of that dispatch
    # is a place a method can be missing: `fold`, `unwrap_or`, `expect` and
    # `parse_int_base` all were, while working in every other position (F54).
    # A range's whole list surface was missing there too (F53).
    "probed": "{calls}\n",
}


PRELUDE = """fn built() -> Map[str, i64] {
    var m: Map[str, i64] = map()
    m.set("a", 1)
    m.set("b", 2)
    m
}

fn empty_map() -> [Map[str, i64]] {
    let one: Map[str, i64] = map()
    [one]
}

fn empty_list() -> [[i64]] {
    let one: [i64] = []
    [one]
}

fn built_unit() -> Map[str, ()] {
    var m: Map[str, ()] = map()
    m.set("a", ())
    m.set("b", ())
    m
}

fn built_tuple() -> Map[str, (i64, str)] {
    var m: Map[str, (i64, str)] = map()
    m.set("a", (1, "x"))
    m.set("b", (2, "y"))
    m
}

"""


def build(ty, lit, calls, shape):
    if shape == "probed":
        # Bare, not interpolated. Interpolating the call puts a *string* in the
        # list, so the backend never has to name the call's own type -- which
        # is the whole point of this shape. Written that way it reported
        # success while the four missing arms it exists for stayed missing.
        #
        # One list per call, because the results do not share a type, and each
        # is read back out so a wrong answer shows and not only a refusal.
        lines = "\n".join(
            f"    let probed{i} = [{c}]\n    print(\"{{probed{i}[0]}}\")"
            for i, c in enumerate(calls)
        )
    else:
        # Every result is interpolated, so one program covers every method and
        # a disagreement names the position.
        lines = ",\n".join(f'        "{{{c}}}"' for c in calls)
    if shape in ("literal", "probed"):
        # Every mention of the receiver, not only the ones a `.` follows:
        # `(R / 4.0).asin()` and `R ?? 0 - 1` name it bare.
        # A function replacement, not a string one. `re.sub` reads escapes in
        # a string replacement, so a literal holding `\t` or `\u{65e5}` --
        # which is what a content axis is made of -- was rewritten into a real
        # tab, or raised "bad escape \u" and took the whole generator down.
        lines = re.sub(r"\bR\b", lambda _m: f"({lit})", lines)
    body = SHAPES[shape].format(lit=lit, calls=lines)
    # An empty literal has nothing to read a type off, so the positions that
    # bind it have to say. `literal` cannot say anything at all -- there is no
    # binding to annotate -- so that one is skipped, and the empty literal is
    # bound through a helper instead. This is the checker being right
    # ([T-EMPTY-LIT]), not the backend refusing.
    if lit in ("map()", "[]"):
        t = ty_decl(ty)
        body = body.replace(f"let R = {lit}", f"let R: {t} = {lit}")
        body = body.replace(f"var R = {lit}", f"var R: {t} = {lit}")
        body = body.replace(f"for R in [{lit}]", f"for R in empty_{ty.split()[1]}()")
        # The list literal's element -- the position F52 was refused in. The
        # holder is what says the type; without it the checker is right to
        # refuse, and the shape would have been dropped rather than run.
        body = body.replace(
            f"let holder = [{lit}]", f"let holder: [{t}] = [{lit}]"
        )
    decl = f"type Box = {{ v: {ty_decl(ty)} }}\n\n" if shape == "field" else ""
    return PRELUDE + decl + "fn main() / {io} {\n" + body + "}\n"


def ty_decl(ty):
    return {
        "str": "str",
        "i64": "i64",
        "f64": "f64",
        "list": "[i64]",
        "bytes": "bytes",
        "str empty": "str",
        "str multibyte": "str",
        "str whitespace": "str",
        "bytes invalid utf8": "bytes",
        "bytes with nul": "bytes",
        "empty map": "Map[str, i64]",
        "empty list": "[i64]",
        "map": "Map[str, i64]",
        "option": "Option[i64]",
        "map of unit": "Map[str, ()]",
        "map of tuple": "Map[str, (i64, str)]",
        "option of unit": "Option[()]",
        "option of tuple": "Option[(i64, str)]",
        "option of list": "Option[[i64]]",
        "list of unit": "[()]",
        "list of tuple": "[(i64, str)]",
        "list of option": "[Option[i64]]",
        "list of bool": "[bool]",
        "list of bytes": "[bytes]",
    }[ty]


def run(cmd, timeout=180):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def spec_methods():
    """Every method the library spec's tables name, so the grid cannot drift."""
    text = open(os.path.join(ROOT, "spec", "library.md")).read()
    return {m.group(2) for m in re.finditer(r"`([a-z]+)\.([a-z_0-9]+)\(", text)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="grasm_fuzz_methods_")
    path = os.path.join(work, "p.gr")
    exe = os.path.join(work, "p.bin")
    checked, disagreed, declined, rejected = 0, [], [], []
    covered = set()
    try:
        for ty, lit, calls in GROUPS:
            for c in calls:
                covered.update(re.findall(r"\.([a-z_0-9]+)\(", c))
            for shape in SHAPES:
                # Nothing binds the receiver in `literal`, so there is nowhere
                # to put the annotation an empty literal needs.
                if lit in ("map()", "[]") and shape in ("literal", "probed"):
                    continue
                src = build(ty, lit, calls, shape)
                with open(path, "w") as f:
                    f.write(src)
                checked += 1
                label = f"{ty}/{shape}"
                rc, out, err = run([GRASM, "run", path])
                if rc != 0:
                    rejected.append(label)
                    print(f"REJECTED {label}: {(out + err).strip()}\n{src}")
                    continue
                want = out
                rc2, _, err2 = run([GRASM, "build", path, "--native", "-o", exe])
                if rc2 != 0:
                    first = err2.strip().splitlines()[0] if err2.strip() else ""
                    # The backend distinguishes these itself: "does not yet
                    # support X" is a named gap, and "codegen bug" is a
                    # verifier failure the compiler is reporting against
                    # itself. Counting the second as a decline is how
                    # `b.is_empty()` failed to compile in every program in the
                    # language while this gate said nothing.
                    if "codegen bug" in first:
                        disagreed.append(label)
                        print(f"CODEGEN BUG {label}: {first}")
                    else:
                        declined.append(f"{label}: {first}")
                    continue
                rc3, got, err3 = run([exe])
                if rc3 == 0 and got == want:
                    # Every method that answers something boxed has to hand it
                    # over exactly once (F57).
                    held = leaked(exe)
                    if held:
                        disagreed.append(label)
                        print(f"LEAKS {label}: {held}")
                if rc3 != 0 or got != want:
                    disagreed.append(label)
                    # Name the position rather than dumping two long lines.
                    w, g = want.strip().split("|"), got.strip().split("|")
                    for i, (a, b) in enumerate(zip(w, g)):
                        if a != b:
                            which = calls[i] if i < len(calls) else "?"
                            print(f"DISAGREES {label} at {which}: run={a!r} native={b!r}")
                    if len(w) != len(g):
                        print(f"DISAGREES {label}: {len(w)} vs {len(g)} results")
        # The mutating methods are covered by `fuzz_places.py`, which crosses
        # them with the place shapes that matter for a write. `f` is the regex
        # reading `x.f` out of the spec's prose, and the rest are free
        # functions rather than methods.
        elsewhere = {
            "push", "pop", "remove", "set", "delete", "sort", "sort_by",
            "map", "ref", "release", "alive", "f",
        }
        missing = sorted(spec_methods() - covered - elsewhere)
        if missing:
            print(f"  NOT COVERED by this grid: {', '.join(missing)}")
        for d in declined:
            print(f"  native declined (not a defect): {d}")
        print(
            f"programs: {checked}   disagreed: {len(disagreed)}   "
            f"rejected: {len(rejected)}   declined: {len(declined)}"
        )
    finally:
        if args.keep:
            print(f"kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)
    # A decline is coverage lost, not an honest no. [§11] says the backend
    # compiles everything the interpreter accepts bar polymorphic recursion,
    # so a refusal here is either a new gap -- which belongs in
    # `scripts/nativegaps.sh` and in the tutorial's table -- or a defect.
    # Counted and not acted on, it reads as PASS while programs quietly stop
    # running: removing the F52 fix left this at `declined: 1` and exit 0.
    sys.exit(1 if disagreed or rejected or declined else 0)


if __name__ == "__main__":
    main()
