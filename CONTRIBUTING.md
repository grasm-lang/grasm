# Contributing

## The one rule that matters

**The interpreter defines the language.** `crates/grasm_interp` is the reference
implementation. Where it and the native backend disagree, the interpreter is
right and the backend has a bug — unless the interpreter contradicts `spec/`,
in which case the specification is right and the interpreter has a bug.

Everything below follows from that.

## Before you open a pull request

```
cargo build --release --workspace
cargo test --workspace                # all tests
cargo clippy --workspace --all-targets   # must be silent
./scripts/difftest.sh                 # native output must equal interpreter output
cargo test -p grasm_spec              # specification conformance
```

CI runs the same four on macOS, Linux, and Windows, plus a cross-compilation
job. A pull request that does not pass locally will not pass there.

## Changing behaviour

If a change alters what a program prints, exits with, or refuses to compile:

1. **Write or update the clause in `spec/`.** Give it a tag. The specification
   is where a decision is recorded; a commit message is not.
2. **Add `spec/conformance/<TAG>.gr`.** Every clause needs a program, and every
   program must name a real clause — both directions are tested, so you cannot
   forget either.
3. **Record its stability level in `spec/stability.md`.** A clause with no level
   fails the suite.
4. **Check both engines.** `./scripts/difftest.sh` compiles every program in the
   repository and compares native output against the interpreter byte for byte.

If the native backend cannot compile something yet, it must say so by name —
`nope(span, "generic functions")` — so difftest reports it as unsupported rather
than passing silently. A skipped program is never counted as a pass.

## Tests

Test the behaviour, not the implementation. A test named
`a_blank_line_ends_a_doc_block` says what the language does;
`test_attach_docs_2` says nothing.

Every bug fix gets a test that fails before the fix. That is not a formality:
several bugs in this repository were fixed twice because the first fix silently
did not apply.

## Comments

Comment the *why*, never the *what*. `// increment i` is noise. A comment
earns its place by explaining a decision, a constraint, or a trap:

```rust
// One scope pop for one push, on the path both branches share.
```

```rust
// `abs` would overflow on i64::MIN.
```

If code needs a comment to explain what it does, rewrite the code.

## Performance claims

Any performance claim needs a reproduction. `scripts/bench.sh` builds every
implementation, verifies its answer against the interpreter, and refuses to time
one that got a different result. Add a benchmark rather than a number in a
commit message.

## Commit messages

Say what changed and why. If a change was surprising — a bug that turned out to
be somewhere other than where it looked, a fix that had to be rethought — say
that too. The commit log is the only place that history survives.

## Scope

Small, complete changes. A pull request that adds a language feature includes
the checker rule, the interpreter, the native backend, the spec clause, the
conformance program, and the tests. One that adds only some of those leaves the
gates able to pass while the language is inconsistent.
