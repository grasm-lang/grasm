# Changelog

Grasm is pre-1.0: minor releases may break anything not marked `stable` in
`spec/stability.md`. See `docs/stability.md`.

## Unreleased (0.9.1)

### Fixed

- **A parallel operation may no longer reach a `ref` from outside** — the race
  the previous entry created. [T-PAR-MAP-PURE] had named the shape and said it
  was "not a race today: the native backend refuses a write through a `ref`
  outright". It stopped refusing, and a program `check` accepted then died with
  SIGTRAP, or panicked inside the runtime with `field access on non-struct`, or
  printed a different wrong answer on each run at exit 0 — several workers
  reaching one slot touch a reference count that is not atomic. Reading was as
  unsafe as writing, because a read takes a count on what it finds. Now refused
  three ways: an element type that reaches a reference, any mention of an
  outside reference inside the function, and a generic carrying one in
  (recorded on the generic's parameter and settled at its call sites, the way
  [T-MAP-KEY-GENERIC] settles map keys). Found independently by both evaluators
  of round 42.
- **`--native` compiles writes through a `ref`.** The backend refused every one
  of them, so §11 listed the shape and two conformance programs ran under the
  interpreter alone. The write now lands in the slab rather than rebuilding the
  referent and storing it back — which a cycle proves is not the same thing:
  `a.out[0].out[0].name` arrives back at `a`, and the outer copy, stored
  afterwards, erased the inner write. `grasm_ref_make_unique` gives the slab
  sole ownership so the existing mutators write in place, and the walk
  short-circuits above the reference, since a chain of no-op write-backs leaked
  one reference per level (the setters skip their release when handed back the
  pointer they already hold). 203 programs identical across all three engines,
  203 leak-free.
- **`--native` compiles returning a closure.** The refusal named the return,
  while what was actually missing was the closure's *parameter* types — which
  the declared return type had already written down. Taking them from there
  compiles the shape, capturing values of any type and capturing a function
  value to call inside. With this and the `ref` writes, **every program in the
  repository compiles natively: 204 identical across all three engines, 0
  unsupported, 0 leaking.**
- **The UFCS help no longer promises a call the compiler will not make.**
  `"hi".print()` was refused with "a free function `fn print(str, ...)` would
  also work here (UFCS)" — exactly the call that had just been refused.
  [L-UFCS] now says the language's own free functions are not methods of any
  type, and the help names the spelling that works: `"hi" |> print`.

## 0.9.0 — 2026-08-16

The first release with a scope. Everything below `1.0` is still pre-1.0 by the
rules in `docs/stability.md`; what changes here is that a version now has a
defined surface and a defined bar, written down in `docs/release-scope.md`.

### The 1.0 surface

**In:** types and inference, functions, pattern matching, collections and the
standard library, modules, fallible values `T ! E`, ownership and `inout`, the
interpreter, the bundle format and the native backend.

**Out, and documented as out:** effects and their rows, the capability ceiling,
parallel purity, regions, Tier-2 `ref`, and FFI. Those clauses are
`experimental` — usable, specified, tested, and not covered by the
compatibility promise.

### What blocks a release

S1 (a crash, a wrong answer, a disagreement between the three engines that
[E-PANIC-TRACE] does not license, or a build that leaks state) and S2 (a
`stable` clause proved unsound) block. S3 and S4 — unsoundness in surface the
specification already labels experimental, over-strictness anywhere, and
diagnostics — are changelog content, and every open one is listed with a
reproducer in `docs/known-issues.md`.

At this release: **no S1 and no S2 are open.** One S3 (`--native` refuses three
shapes the interpreter accepts, all listed in §11 and gate-proven) and one S4
(the UFCS help text promises a call the compiler will not make).

### Effect rows come from the type, not from a guess

The effect pass used to reconstruct, syntactically, a fact the type checker
already had: which function value a name holds, and therefore what calling it
performs. It did that with a name-to-sources map, a pool of every function
value a body mentions, and a special case per binder — `let`, `for`, a `match`
payload, a tuple destructuring, an index, a field, `??`, a block, a closure
parameter. Every missing cell of that grid was a defect, and ten rounds of
cold-model evaluation spent most of their findings there.

A function's row is part of its type and travels through every carrier by
unification, so the checker now publishes the row of each function-valued
expression and of each call whose callee is not an expression of its own, and
the effect pass reads it. **511 lines of guessing deleted**, and the class of
defect they produced no longer has a mechanism.

The deletion was made on evidence: the whole gate set was run first with the
syntactic path switched off, and only what it caught was removed.

### Gates

201 conformance and example programs identical across `grasm run`, the bundle
and the native binary; 16 three-engine agreement programs; 201 leak-free
builds; the §11 gap list proved to match the compiler in both directions; and
five generated-program gates — effect rows (204 programs, both directions,
including across a package boundary), FFI spellings (36), effect handlers,
differential, fallible values and generic instantiation. All of it now runs in
CI rather than by hand.

### Also in this release

- **`scripts/fuzz_extern.py`** — every spelling by which a C function can be
  named or called, each checked against a row missing `io` (must fail) and
  against the complete row (must compile and run).
- **[T-EFFECT-OP-RESULT]** — an effect operation's declared result carries its
  row. An operation answering `-> fn(i64) -> i64 / {io}` handed out a printing
  function that nothing charged.
- Scope decides for a **module alias** and an **effect name**, as it already
  did for a function name.
- Five clauses demoted from `stable` to `experimental`
  (`T-FN-TYPE-ROW`, `T-FN-TYPE-ROW-PARAM`, `T-FN-TYPE-ROW-CARRIED`,
  `T-FN-VALUE-ROW`, `T-UNSAFE-UFCS`): their implementation changed in nearly
  every evaluation round, which is what the promotion criteria forbid.

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

- **`fold`, `entries`, `expect`.** The three gaps cold-model runs kept hitting.
  `xs.fold(init, f)` is the one combinator whose result is not a function of
  the element type — `[i64].fold("", ..)` is a `str`. `m.entries()` returns
  `[(K, V)]` in key order, so walking a map no longer means iterating `keys()`
  and looking each one back up. `o.expect(msg)` is the escape hatch for
  absence being a bug rather than a case; the message is required, because a
  panic nobody can read is not an improvement on the `??` it replaces.
- **`panic` accepts a computed message.** Natively it took a literal only, so
  `panic("value {n} is out of range")` did not compile.
- **Function types.** `fn(A, B) -> R` may be written wherever a type may be,
  so a program can declare its own higher-order function — `fn apply(f: fn(i64)
  -> i64, x: i64)` — instead of being limited to the built-in combinators. A
  closure passed to such a parameter takes its parameter types from it. The
  type carries an effect row and **no row means the empty row**, so
  `fn(i64) -> i64` accepts only functions that perform nothing and an effectful
  one is rejected where it is passed. That check is what stops a function value
  from being a way to carry an effect past a declared row.
- **`grasm publish` and `grasm add`.** `publish` copies a package into a
  registry after checking it against its own ceiling, and **refuses to
  overwrite a version that already exists** — a published version is
  immutable, which is what makes the lockfile's checksum mean something.
  `add` appends a dependency to `grasm.toml` without reformatting the file.
- **[L-DISPLAY] and [L-UFCS].** How every type renders, because a program's
  output is what its correctness is judged on; and that a free function whose
  first parameter accepts the receiver may be called as `x.f(y)`.
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
- **The parallel-purity proof only checked closures written at the call site.**
  `xs.par_map(announce)` where `announce` prints compiled and ran across
  threads — including with `/ {io}` declared on it. The check walked the
  argument as an expression, and naming a value performs nothing; what counts
  is the row of the function about to run on several cores. Named functions,
  named closures, and effects reached transitively are all caught now.
- **`grasm deps` exited 0 for a package it could not analyze**, omitting it
  from the capability table and writing the lockfile. A dependency that cannot
  be checked has not passed; it is an error and no lockfile is written.
- Tuples printed as `[1, one, true]` natively and `(1, one, true)` under the
  interpreter: the runtime had no tuple, so the backend built one as a list.
  Found by writing [L-DISPLAY], which specifies what `print` produces for
  every type.
- A failed import named one of the two places it looked, and named the one it
  did not want. It lists both now.
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
