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
- **`grasm deps --locked`** fails when anything differs from `grasm.lock`,
  including a widened capability ceiling, which is reported separately from a
  content change.

- **The native backend compiles structured concurrency and generic functions.**
  `parallel`/`spawn` lower to a recorded list of spawn-site indices dispatched
  after the block, so ordering and joining match the interpreter without
  closures or environment capture. Generic functions are monomorphized per
  instantiation, including across module boundaries.
- **The native backend compiles function values.** A named function used as
  data, a closure bound to a name, closures capturing their environment, and
  lists of either — all dispatched through an indirect call. Native coverage
  goes from 103 programs to **113 of 113: nothing in the repository is left to
  the interpreter.**

- **Unboxed scalar collections.** `[i64]`, `[f64]`, and `[bool]` store
  elements inline — one word each, no per-element heap cell, no refcount
  traffic. sieve 89→58ms, matmul 187→92ms, sort 97→49ms (now faster than C
  and Go). Representation only; `[E-COW]` says it is never observable, and
  difftest holds all 115 programs byte-identical across both engines.

- **Native binaries dead-strip the runtime.** `-Wl,-dead_strip` (macOS) /
  `--gc-sections` (Linux) at link time: 1,503 KB → ~442 KB mean, matching
  Rust. The rest is `std` machinery; going lower needs `no_std` or
  `build-std`.

- **`par_map` — multi-core, deterministic.** `xs.par_map(f)` spreads elements
  across cores: **5.9× on 8 cores**, byte-identical output. Safety comes from
  the effect system — `f` must be effect-free and the compiler proves it
  ([T-PAR-MAP-PURE]) — and results are returned in input order regardless of
  core count ([E-PAR-MAP-ORDER]). `parallel`/`spawn` stays single-core by
  design; its tasks share scope and would race.

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
- The lockfile reader collapsed `[[package]]` into a single table, so
  verification compared one package and ignored the rest.
- A task body could reference a binding declared inside its `parallel` block —
  a loop variable, say. The checker accepted it and the interpreter panicked at
  run time with "unknown name", because tasks run after the block's scope is
  gone. It is now a compile error ([T-TASK-CAPTURE]).

### Changed

- `scripts/difftest.sh` covers `spec/conformance/` and `benchmarks/` as well as
  `examples/` and `projects/` — 19 programs to 103. It compares stdout and exit
  codes strictly; stderr is compared with the interpreter-only panic trace block
  excluded, a gap recorded in the README rather than hidden.
