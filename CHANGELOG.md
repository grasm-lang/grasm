# Changelog

Grasm is pre-1.0: minor releases may break anything not marked `stable` in
`spec/stability.md`. See `docs/stability.md`.

## Unreleased

### Added

- **Property tests and benchmarks.** `test "name" (xs: [i64]) { ... }` generates
  its inputs; parameters are the only difference from a unit test, so fuzzing is
  the same machinery pointed at `str`. Failures report a shrunk counterexample
  and the seed that found it. `bench "name" { ... }` reports ns/op via
  `grasm bench`. `grasm test --coverage` reports statement-line coverage.
- **Documentation.** `///` comments document the declaration that follows;
  `grasm doc` renders Markdown with signatures taken from the AST. Fenced blocks
  in doc comments are executed by `grasm test`. `docs/tutorial.md` is a guided
  tour whose code the test suite compiles and runs.
- **Debugging.** Panics print a stack trace. `grasm dap` is a Debug Adapter
  Protocol server: breakpoints, stepping, frames, locals, variable lookup.
- **Standard library.** `any` / `all` / `find` / `min` / `max` / `is_empty` on
  lists, `lines` / `pad_start` / `pad_end` / `is_empty` on strings, `is_empty`
  on maps, and `now_millis` / `env` / `exit`.
- **Benchmarks.** Five programs in Grasm, C, Rust, Go, and Python, with a
  harness that verifies answers before reporting timings.
- **Project documents.** LICENSE (Apache-2.0), NOTICE, CONTRIBUTING,
  GOVERNANCE, SECURITY, CODE_OF_CONDUCT, and this file.

### Fixed

- `exit(n)` called `std::process::exit` from inside the interpreter, killing any
  embedder — the test harness died with status 3 instead of reporting a failure.
- The native map scanned its sorted entries linearly, making every map operation
  O(n) and whole programs quadratic. `wordfreq` was slower natively than in the
  interpreter (1430ms vs 460ms); binary search took it to 183ms.
- Boxing a `bool` or small `i64` allocated, so a sieve allocated once per flag
  write. Those values are now interned. `sieve`: 538ms → 84ms.
- `bench` with no name hung the parser: declaration recovery broke on the same
  token `parse_decl` had rejected. Recovery now always consumes something.
- `pad_start` / `pad_end` silently used only a multi-character fill's first
  character, and substituted a space for an empty fill.

### Changed

- `scripts/difftest.sh` covers `spec/conformance/` and `benchmarks/` as well as
  `examples/` and `projects/` — 19 programs to 103. It compares stdout and exit
  codes strictly; stderr is compared with the interpreter-only panic trace block
  excluded, a gap recorded in the README rather than hidden.
