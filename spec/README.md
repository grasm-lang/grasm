# The Grasm Language Specification

This directory is the normative description of Grasm-Core.

| File | Contents |
|---|---|
| [`core.md`](core.md) | The Core/Full split: which forms are the kernel, and the rewrite for every form that is not |
| [`grammar.md`](grammar.md) | Lexical structure and EBNF grammar |
| [`types.md`](types.md) | Type system, inference, and subtyping rules |
| [`semantics.md`](semantics.md) | Evaluation, ownership, effects, and concurrency |
| `conformance/` | One `.gr` program per normative clause |

## How this specification is kept honest

Every normative rule carries a stable tag such as **[E-ARITH-INT]**. For each
tag there is a conformance program under `conformance/` whose name begins with
that tag, and `cargo test -p grasm_spec` fails if:

1. a tagged rule has no conformance program, or
2. a conformance program's actual behavior differs from the `//! expect:`
   header it declares.

So the specification cannot drift from the implementation without a red test.

Run the suite:

```
cargo test -p grasm_spec
```

## Status

This directory describes the language the implementation supports today. The
**Grasm-Core / Grasm-Full split** that MASTERPLAN §19.2 makes binding is defined
in [`core.md`](core.md), with a rewrite for every Full form and a test that fails
when a new syntactic form is classified as neither.

That document also records what is *not* met: Core is 37 forms against a budget
of 30, the reference interpreter is 6,009 lines against 5,000, and this
specification is about 55 pages against 50. Those three numbers were asserted as
met for a long time before anybody measured them. It is not yet a frozen 1.0 specification; see
`STABILITY.md` in the repository root for what stability guarantees exist.

## Conformance program format

```grasm
//! tag: E-ARITH-INT
//! expect: output
//! 7
fn main() {
    print("{1 + 2 * 3}")
}
```

Headers:

- `//! tag:` — the normative clause this program exercises (required)
- `//! expect: output` — the following `//!` lines are the exact expected stdout
  (and the program must exit 0)
- `//! expect: output-failing` — as above, but the program must exit non-zero
- `//! expect: error` — the program must fail to compile; following lines are
  substrings that must appear in the diagnostics
- `//! expect: panic` — the program must compile and then panic at runtime;
  following lines are substrings of the panic message
