#!/usr/bin/env python3
"""Writes tools/tirinterp/src/ccall.gr: one `call_c` site per C call shape.

The TIR interpreter calls a C function the way `grasm run` does: it finds the
symbol in its own process and calls it with the shape the `extern fn`
declares ([L-LIB-CALL-C]). Grasm has no variadic call, so each shape needs a
call site with a signature of its own, and this writes them out.

The shapes are exactly the ones `crates/grasm_interp/src/ffi.rs` can call, so
a shape outside them panics in both interpreters with the same words. In
`ffi.rs` a shape is over two register classes, an integer word and an `f64`;
here an integer word is one of three Grasm types -- an integer or address,
a `str` (crossing as a NUL-terminated copy) or `bytes` (as its buffer) -- so
each integer slot is written three times.

    python3 scripts/tirinterp_ccall.py          writes the file
    python3 scripts/tirinterp_ccall.py --check  fails if it is stale
"""
import itertools
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "tools/tirinterp/src/ccall.gr"

# ffi.rs's shapes: (parameter classes, the results each may have).
# `I` is an integer word, `F` an f64; `i`, `f`, `v` are an integer, an f64
# and no result.
SHAPES = [
    ("", "vif"),
    ("I", "vif"),
    ("F", "vif"),
    ("II", "ifv"),
    ("IF", "if"),
    ("FI", "if"),
    ("FF", "ifv"),
    ("III", "i"),
    ("FFF", "f"),
]

GRASM = {"i": "i64", "s": "str", "b": "bytes", "f": "f64"}
TAKE = {"i": "iv", "s": "sv", "b": "bv", "f": "fv"}


def kinds(classes):
    """Each parameter class as the Grasm kinds it may be."""
    choices = [("i", "s", "b") if c == "I" else ("f",) for c in classes]
    return ["".join(k) for k in itertools.product(*choices)]


def render():
    sites = []
    for classes, rets in SHAPES:
        for k in kinds(classes):
            for r in rets:
                sites.append((k, r))
    out = []
    out.append("// Calls a C function at an address with a shape known only when the")
    out.append("// program runs ([L-LIB-CALL-C]), by choosing among call sites that each")
    out.append("// have one: Grasm has no variadic call. The shapes are exactly those")
    out.append("// `crates/grasm_interp/src/ffi.rs` can call, so one outside them is refused")
    out.append("// in both interpreters in the same words.")
    out.append("//")
    out.append("// Written by scripts/tirinterp_ccall.py; edit that, not this.")
    out.append("import value")
    out.append("")
    for k, r in sites:
        params = ", ".join(GRASM[c] for c in k)
        ret = {"i": " -> i64", "f": " -> f64", "v": ""}[r]
        out.append(f"type C_{k or '0'}_{r} = fn({params}){ret}")
    out.append("")
    out.append("fn iv(v: Value) -> i64 {")
    out.append("    match v {")
    out.append("        VInt(n) -> n")
    out.append("        VU64(u) -> u.as_i64()")
    out.append("        _ -> 0")
    out.append("    }")
    out.append("}")
    out.append("")
    out.append("fn fv(v: Value) -> f64 {")
    out.append("    match v {")
    out.append("        VFloat(f) -> f")
    out.append("        _ -> 0.0")
    out.append("    }")
    out.append("}")
    out.append("")
    out.append("fn sv(v: Value) -> str {")
    out.append("    match v {")
    out.append("        VStr(s) -> s")
    out.append('        _ -> ""')
    out.append("    }")
    out.append("}")
    out.append("")
    out.append("fn bv(v: Value) -> bytes {")
    out.append("    match v {")
    out.append("        VBytes(b) -> b")
    out.append("        _ -> bytes_of([])")
    out.append("    }")
    out.append("}")
    out.append("")
    out.append("/// Calls `f` with `args`, whose kinds are `ks` -- a letter each: `i` an")
    out.append("/// integer or address, `s` a `str`, `b` `bytes`, `f` an `f64` -- for a")
    out.append("/// result `r`: `i` an integer word, `f` an `f64`, `v` none. `None` when no")
    out.append("/// call site has the shape.")
    out.append("pub fn call(f: addr, ks: str, r: str, args: [Value]) -> Option[Value] / {io, mem, unsafe} {")
    out.append("    let a = args")
    out.append("    unsafe {")
    out.append('        match ks + ">" + r {')
    for k, r in sites:
        alias = f"C_{k or '0'}_{r}"
        vals = "".join(f", {TAKE[c]}(a[{i}])" for i, c in enumerate(k))
        call = f"f.call_c({alias}{vals})"
        key = f'"{k}>{r}"'
        if r == "i":
            out.append(f"            {key} -> Some(VInt({call}))")
        elif r == "f":
            out.append(f"            {key} -> Some(VFloat({call}))")
        else:
            out.append(f"            {key} -> {{")
            out.append(f"                {call}")
            out.append("                Some(VUnit)")
            out.append("            }")
    out.append("            _ -> None")
    out.append("        }")
    out.append("    }")
    out.append("}")
    return "\n".join(out) + "\n", len(sites)


def main():
    text, n = render()
    if "--check" in sys.argv[1:]:
        if not OUT.exists() or OUT.read_text() != text:
            print(f"{OUT.relative_to(ROOT)} is stale: run scripts/tirinterp_ccall.py")
            sys.exit(1)
        print(f"{OUT.relative_to(ROOT)}: {n} call shapes, current")
        return
    OUT.write_text(text)
    print(f"wrote {OUT.relative_to(ROOT)}: {n} call shapes")


if __name__ == "__main__":
    main()
