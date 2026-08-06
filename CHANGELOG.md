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
  goes from 103 programs to **all of them: nothing in the repository is left
  to the interpreter.**

- **Unboxed scalar collections.** `[i64]`, `[f64]`, and `[bool]` store
  elements inline — one word each, no per-element heap cell, no refcount
  traffic. sieve 89→58ms, matmul 187→92ms, sort 97→49ms (now faster than C
  and Go). Representation only; `[E-COW]` says it is never observable, and
  difftest holds every program byte-identical across both engines.

- **Native binaries dead-strip the runtime.** `-Wl,-dead_strip` (macOS) /
  `--gc-sections` (Linux) at link time: 1,503 KB → ~442 KB mean, matching
  Rust. The rest is `std` machinery; going lower needs `no_std` or
  `build-std`.

- **`par_filter` — the other half of effect-proven parallelism.** The predicate
  runs across cores into a mask; compaction then walks the mask in order, so
  the output is the same whatever the core count is ([E-PAR-FILTER-ORDER]).

- **Version requirements on dependencies.** `version = "^0.3.1"` accepts that
  version or any later one that does not change the leading non-zero number;
  `=0.3.1` accepts exactly it; bare `0.3.1` means `^0.3.1`. Before 1.0 the
  minor number carries the weight of a major, the same rule the stability
  policy states for the language.

- **A registry, and minimal version selection.** A dependency with a `version`
  and no `path` is resolved from a directory of `<name>/<version>/` packages
  (`GRASM_REGISTRY`, or the nearest `.grasm/registry`). Each package gets the
  highest floor any requirement stated, resolved to the lowest version present
  at or above it, then checked against every requirement. No search and no
  backtracking, so the answer does not depend on walk order and adding a
  dependency cannot silently upgrade one you already had; requirements that
  cannot all hold are an error naming each requirer. The capability ceiling
  now covers code you did not write: a registry package exceeding its declared
  effects fails every consumer's build, and contents changed under a fixed
  version number fail `grasm deps --locked`.

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

- Element access in a hot loop went through the runtime on every read and
  write. A list a loop body only ever indexes is now pinned for the loop and
  reached through its data pointer. The first round left a refcount round trip
  on each read — an increment and a matching decrement around a value the load
  never used, four indirect calls per iteration of `matmul`'s inner loop.
  `matmul` 99ms → 61ms → **30ms**, against C's 25ms.
- Maps scanned a sorted vector; they are hash tables now, and iteration sorts,
  so `[E-MAP-ORDER]` is paid once per iteration rather than on every lookup.
  A lookup also no longer copies its key onto the heap to look at it.
- `RtPayload::Variant` held its names as `String`, so every `Some(x)` — every
  map lookup builds one — allocated a private copy of the word "Some". Names
  are static data and are borrowed now.
- Scalar list iteration returned a boxed trait object, so summing a list paid
  a dynamic dispatch per element. Now 0.1ns/element.
- Two directories claiming the same package name resolved to whichever the
  walk reached first, silently, and the other dependent linked something it
  never asked for. It is an error naming both directories, both versions, and
  who required each.
- The native backend's static type inference — used where no annotation
  exists, most sharply for a list literal's element type — knew four builtin
  methods. Every other one failed there, so `[xs.min() ?? 0]` ran under the
  interpreter and would not compile natively while `xs.min() ?? 0` alone
  compiled fine. Found by `scripts/fuzz_differential.py`, which generates
  well-typed programs and compares both engines; the 130-program differential
  gate had missed it because none of those puts a reduction inside a list
  literal.
- `scripts/crosstest.sh` counted a link failure as "unsupported", so a runtime
  archive older than its own source read as a backend limitation and cost
  seven programs of coverage without a word. It now refuses a stale archive up
  front and names every skip with the step that gave up.

### Changed

- `scripts/difftest.sh` covers `spec/conformance/` and `benchmarks/` as well as
  `examples/` and `projects/` — 19 programs to 128. It compares stdout and exit
  codes strictly; stderr is compared with the interpreter-only panic trace block
  excluded, a gap recorded in the README rather than hidden.
