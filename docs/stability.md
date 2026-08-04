# Stability policy

Grasm is pre-1.0. This document says exactly what that means, so that "we might
change things" is a bounded promise rather than an excuse.

The unit of stability is the **normative clause**: every rule in `spec/` carries
a tag like `[E-MAP-ORDER]`, and every tag has a stability level recorded in
`spec/stability.md`. A clause with no recorded level fails the test suite, so
this document cannot drift out of sync with the specification the way a
hand-maintained list would.

## Levels

**`stable`** — will not change incompatibly before 2.0. A program relying on a
stable clause keeps working. Fixing an implementation that *disagrees* with a
stable clause is not a breaking change; the clause is the contract, not the bug.

**`provisional`** — the behaviour is specified and tested, but the design is
still being validated against real programs. It may change in a minor release
with a migration note in the changelog. Most of the language is here today.

**`experimental`** — may change or be removed in any release, without a
deprecation period. New surface starts here.

## What "breaking" means

A change is breaking if a program that compiled and ran correctly before now
fails to compile, or produces different output. By that definition these are
**not** breaking:

- Making the compiler accept a program it used to reject.
- Making a *diagnostic* clearer, or changing its wording.
- Changing performance, memory use, or the machine code emitted.
- Changing anything under `crates/` that is not reachable from the CLI or the
  specification. The Rust crates are implementation detail, not a public API.
- Fixing the interpreter or the native backend to match a stable clause.

These are:

- Removing or renaming a builtin, method, keyword, or CLI flag.
- Changing the meaning of a stable clause.
- Changing the output of a program that only uses stable behaviour.

## Versioning

`MAJOR.MINOR.PATCH`, and the version applies to the toolchain as a whole — the
language, the CLI, the specification, and the file formats (`grasm.toml`,
`grasm.lock`) move together. Splitting them would let the language and the tool
that implements it drift apart, which is the problem the conformance suite
exists to prevent.

- **PATCH** — bug fixes; no clause changes level or meaning.
- **MINOR** — new clauses, new builtins, new CLI surface. Provisional and
  experimental clauses may change here, with a changelog entry.
- **MAJOR** — stable clauses may change. Before 1.0, the MINOR number carries
  that weight instead: 0.x → 0.(x+1) may break anything.

## Reaching 1.0

1.0 is not a date, it is a checklist:

1. Every clause in `spec/` is `stable` or deliberately, documentedly not.
2. The native backend compiles every program the interpreter runs — the one
   remaining gap is function values (passing a function as data), and
   `scripts/difftest.sh` reports it by name rather than skipping quietly.
3. `parallel` runs on multiple cores. Today it is deterministic and
   single-threaded, which is stated in `[E-NOT-PARALLEL]` and is a
   specification of the current implementation, not a design goal.
4. The package manager resolves versions, not just paths.
5. A written migration story for each provisional clause that changed on the
   way there.

## Deprecation

Removing something stable takes two minor releases: first the compiler warns
and the changelog says what to use instead; then it goes. Nothing stable is
removed without a warning release in between.

## Security

See `SECURITY.md`. Security fixes ship in a PATCH release and may change
behaviour that a stable clause describes if the clause itself is the problem —
that case is called out explicitly in the changelog rather than folded in.
