# Changelog

Grasm is pre-1.0: minor releases may break anything not marked `stable` in
`spec/stability.md`. See `docs/stability.md`.

## Unreleased (0.9.1)

### Fixed

- Interpolating a unit compiles natively. It worked under a type
  annotation and was refused without one; both spellings now render `()`
  (F45).
- **`--native` compiles a generic struct declaration.**
  `type Pair[A, B] = { fst: A, snd: B }`, nested, in lists, through generic
  functions, compared, mutated and printed. A generic type is kept whole until a
  use site fixes its parameters — the same treatment a generic function gets —
  and each member is registered under a name built from its arguments. A literal
  reads those arguments off the fields it gives, the same unification a generic
  call does on its arguments.

  The mangled name is how two members are told apart and is **not** what the
  type is called: it stops at the value's edge, so `print` shows `Pair{..}`
  rather than `Pair[i64,str]{..}`.

- **`--native` compiles a generic sum type.** `type Opt[T] = | Has(v: T) |
  Nothing`, including two instantiations in one program. A constructor fixes the
  sum's parameters from its arguments, the way a struct literal fixes a
  struct's from its fields.

  A type parameter no argument mentions still needs an annotation: `Good(1)` for
  `Res[T, E]` says what `T` is and nothing about `E`, so one compiled copy
  cannot be chosen. The refusal names the parameter.
- **`--native` compiles a UFCS call of a generic free function.**
  `[1, 2].describe()` for `fn describe[T](xs: [T])`. A generic function has no
  symbol until a call site fixes its type parameters, and the lookup this path
  used does not carry generics — so the call was refused while
  `describe([1, 2])` compiled. The receiver is the first parameter ([L-UFCS]),
  which is exactly where the type parameters are read from.

  This also removes a refusal that had been *correct*: a generic free function
  with the same name as a function-typed field. The checker resolves it to the
  function; the backend could not see it, and calling the field printed a heap
  address and multiplied it by two. It resolves the same way as the checker now,
  and both directions of that crossing are gated.

  Three refusals remain in §11, down from seventeen.
- **`--native` compiles a handler that outlives the expression that wrote it.**
  Returned from a function, stored in a struct field, held in a list, walked by
  a `for`, printed, instantiated generically, installed inside `parallel` — six
  shapes, one change. A handler value is reference-counted and its operations'
  captured environments are on the heap, so the question *is this environment
  still alive?* has the same answer as for every other value. A handler written
  directly at a `with` never builds one: its operations are lifted at compile
  time and its captures live in the frame that ran the `with`, and that path is
  untouched.

  Four refusals remain in §11, down from seventeen this morning.
- **`--native` compiles an effect handler passed as a parameter.**
  `fn run(h: LogHandler) { with Log handled_by h { … } }` — the shape
  [E-HANDLER-OWN-ROW] documents. Smaller than it looked: effect *operation*
  calls were already fully dynamic, a lookup by effect and operation name
  followed by an indirect call, so nothing about dispatch changed. What was
  missing was a way to *carry* a handler, because operations are lifted from a
  literal at compile time and a `with` on a parameter has no literal to lift. A
  handler value is now a block of `(code, env)` pairs built in the frame that
  wrote the literal — which is exactly why one may be passed down, and still may
  not be returned or stored in a struct: those outlive the frame, and stay
  refused by name.
- **`--native` compiles a type parameter that appears only in a function-typed
  parameter's result.** `fn key_by[T, K](xs: [T], f: fn(T) -> K) -> [K]`.
  Monomorphization binds a type parameter by matching each declared parameter
  against the argument's actual type, and that matcher had no case for a
  *function* type — so `T` was bound from `xs` and `K` was determined by
  nothing, though the closure passed for `f` had always known its own type.
  `ref T` and `T ! E` were missing for the same reason and are handled now too.
- **`--native` compiles a closure whose parameter types nothing local fixes.**
  `let f = |p| p.0 + p.1`, called as `f((i, v))` in a later statement. Such a
  closure learns its parameters from its call site and inference reaches both;
  the backend was reconstructing that from the shapes visible *at the closure*,
  which cannot reach a call site in another statement. The checker now exports
  what it settled (`closure_param_types`), and the backend asks before
  reconstructing. The reconstruction stays as the fallback, for closures
  inference genuinely leaves unresolved — and those are refused by the checker
  itself, so every engine refuses them with one message.
- **`--native` compiles a `fold` seeded with an unannotated `map()`.** The
  cause was asymmetry rather than difficulty: an empty *list* asked the checker
  what it had resolved the literal to and an empty *map* did not, though the
  checker records both ([T-EMPTY-LIT]). So the seed arrived as a map with
  unknown key and value, and the refusal said the seed and the function differed
  when they were written identically.
- **`--native` compiles a call through a callee that is not a name.**
  `TABLE[0](21)`, `fs[0](3)`, `b.f(6)` where `f` is a struct field of function
  type. The callee is evaluated **once**, bound to a name the source cannot
  spell, and called by the path a named function value already used — bound
  rather than called in place because that path reads the value twice, once for
  the code pointer and once for the environment, and evaluating the callee twice
  would run its side effects twice.
- **`--native` compiles `==` on two bare `ref`s.** A `ref` is a slot and a
  generation packed into one word, and two are equal when they name the same
  slot at the same generation — identity, which is what the interpreter answers
  and the only thing a reference can usefully be asked. So the comparison is the
  word. It compiled for a *list* of refs already, because the runtime's deep
  equality reaches the same answer element by element; only two bare ones had
  nowhere to go. Slot reuse after `release` agrees too, which is what the
  generation is for.
- **`--native` compiles `map` with a function that returns nothing.** The
  refusal was stale in one half and advice in the other. `RtPayload::Unit`
  exists precisely so a list of nothings prints as `[(), ()]` rather than
  `[0, 0]`, so there was a representation; the rest of the message — *write a
  `for` loop, which is what a list of nothings was standing in for* — is good
  advice, and advice that makes two engines disagree about which programs exist
  is worse than no advice. Deleting the refusal was the whole fix; three
  engines agree, including on printing the list.
- **`--native` compiles a range held as a value.** `let r = 0..n`, passed,
  printed, compared, walked. `for i in 0..n` written literally is compiled into
  a counted loop and never builds a range at all — which is why it costs
  nothing, and it is unchanged — so there had been no range value for a binding
  to hold. One exists now: three scalars, so `0..2_000_000` costs what `0..2`
  does. It prints as its bounds rather than its elements, and two compare equal
  when bounds and inclusiveness match, so `0..0` is not equal to `5..5` though
  both are empty. `len` is answered from the bounds without building anything.
- **`--native` compiles a match on `IoError`'s variants** (F38). `IoError` is in
  the prelude — nobody declares it in source — and the backend built its type
  table by walking each module's declarations, so there was never anything to
  find. Telling a missing file from a broken one was a build-time refusal, in a
  language where every `io` call hands one of these back. The variant list is
  now read from the checker's own table rather than copied, so a third variant
  cannot make the two disagree again. Two gaps for one fix: `let r: str !
  IoError = read_file(p)` compiles too.
- **A recorded `()` came back as `0` on another engine** (F37). The compiled
  runtime answered `write_file`, `write_bytes`, `make_dir`, `remove_file` and
  both socket writes with `Ok(0)` where the interpreter answers `Ok(())`. No
  direct run could tell them apart — the static type is `()`, and a compiled
  build prints unit from the type rather than from the value — but a journal is
  exactly a value leaving the engine that made it, and a cross-engine replay
  printed `0` and `[0, 0]`. `RtPayload::Unit` already existed for this; six call
  sites had not been converted.
- **Every gate now builds what it tests** (F36). `scripts/wasmtest.sh` read a
  prebuilt wasm module it never built, so when the wasm target stopped
  compiling it went on reporting `identical: 182` against a module from the
  previous week — for two full runs. The audit that followed found the same
  shape in all nine gates: each opened `./target/release/grasm` without
  building it. A gate reporting a number is making a claim about the code as it
  stands, and reading an artefact it did not build is the one way to make that
  claim without checking it.

- **`grasm check` no longer takes exponential time on a chain of method calls**
  either. The same defect as the field chain, one path over: a method call
  infers its receiver to resolve the method, and `check_fn_call` then checks
  that same receiver against the parameter it fills. Twenty `.bump(1)`s took
  0.7 seconds and forty never finished. Inference is now memoized for the
  shapes that cannot hold a closure — narrow deliberately, since a closure's
  type depends on the position it fills and its body is checked against the
  enclosing function's error type, so for those a second visit is not a repeat.
  Found by auditing for siblings of the field-chain defect rather than by an
  evaluator.
- **`grasm check` no longer takes exponential time on a chain of field reads.**
  `infer_field` inferred its receiver twice — once for the handler-operation
  check, once for the read — so cost doubled per link: twenty-two `.next`s took
  1.3 seconds, twenty-six took twenty-one, and forty never finished, at one
  core and 13 MB. Not a stack overflow and not the nesting cap, which
  [G-NEST-LIMIT] says must *report* rather than hang. A thousand links now
  check instantly.
- **A `par_map` inside a closure emits its worker.** The worklists are drained
  to a fixpoint because compiling one body discovers another; the parallel
  workers were drained once, before that loop, so a `par_map` written inside a
  closure or a handler operation had its lifted worker declared and never
  defined — and the *linker* reported it as a missing C function `gp_main_1`,
  for a program containing no `extern fn`. The identical defect the loop
  already existed to prevent for closures.
- **A struct field of function type is reachable again.** [L-UFCS] takes a free
  function in method position only when "its first parameter accepts the
  receiver", and only the name was compared: declaring `fn len(xs: [i64])`
  anywhere in the module made every `n.len()` on a struct with a `len` field a
  type error naming `[i64]`. Worse, the engines resolve by name and had no way
  to know better, so once the checker was fixed they still called the free
  function: the checker now records which calls it resolved to a field, and
  both engines read that.
- **`--native` compiles two more shapes**, and §11 loses two rows: a parallel
  operation given a function *value* (a `fn`-typed parameter, or `let f = sq`),
  which refused with a message about a closure in programs containing none; and
  a closure literal in a `match` or `if` arm whose binding is annotated, which
  refused with "no type says what its parameters are" about a binding that says
  exactly that. The annotation now reaches the arms.
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

### Fixed — verification of the day's changes

- **The backend resolved a method call differently from the checker, and
  printed a heap address.** The checker records which method calls it resolved
  to a struct field; the backend read that table only to *suppress* its
  field-call path, which is half a rule. It resolves a free function with a
  lookup that does not carry generics, so a *generic* free function sharing a
  name with a function-typed field left the lookup empty and the call fell
  through to the field — while the checker had resolved it to the function.
  `grasm run` printed `77`; the compiled binary printed a string and then
  multiplied its heap address by two. Exit 0, nothing on stderr, a different
  number every run. The table is authoritative in both directions now: it
  suppresses and it permits, and a call the backend cannot resolve the checker's
  way is refused by name, as §11 promises.
- **A field reached through a `ref` was refused** when a same-named free
  function could not accept the receiver. Reading through a reference is
  [T-REF-READ]; being *passed* as one is not, and `fn step(b: Box, …)` cannot
  take a `ref Box`. Treating them as interchangeable made that function a UFCS
  candidate which then failed its own argument check, so the call reported
  "expected `Box`, found `ref Box`" instead of falling back to the field.

### Fixed — the round-44 list

- **An incremental `--native` rebuild shipped a stale imported constant.** A
  module's cached object was keyed on its own source and on the program's
  *signature* surface — and a signature omits a constant's value, correctly, since
  a constant changing value is not an API break. A cache key is wrong to omit it:
  [E-CONST] makes a module-level `let` one value for the whole program, the
  importer's object materialises it, and changing `pub let K = 10` to `999` left
  the importer's entry valid. Bump a dependency, rebuild, ship the old value —
  across a package boundary too. Now keyed on a fingerprint of everything the
  object read, which is the rule rustc's incremental engine uses.
- **An effect operation's result was not row-checked.** `check_expr` returned
  early for a closure once its *own* row was checked, so rows nested in its
  result type never were — every other expression got that check one line
  further down. `MkHandler{ make: || shout }` for `fn make() -> fn(i64) -> i64`
  handed a printing function through an empty-row result, so a function
  declaring `/ {Mk}` printed and a `capabilities = []` package reached `io`,
  while the same signature written as an ordinary `fn` was refused.
- **The registry was found only from one directory.** [T-MODULE-DEP] says "the
  nearest enclosing `.grasm/registry`" and the walk was written to do that — but
  it walked by `parent()` from a relative path, which has no parents. The same
  package built or did not build depending on where the shell was, and the error
  told the reader to put the registry beside the package, where it already was.
- **A deeply nested tuple literal cost O(n³) to check** — 1000 levels took 7.8
  seconds, now 31 ms. `record_value_row` runs on every expression and copied the
  whole type to look at its head.
- **Two specification statements were false.** [L-LIB-SORT-BY] said a sort key
  "must be `i64`, `f64` or `str`" while tuple keys worked in all three engines
  and [L-LIB-TUPLE] said so; [T-REF-WRITE] said the compiled backend refuses
  writes through a reference, one day after it stopped.

- **A compiled binary is 25% smaller** — 437 KB to 328 KB, against [G5]'s 300 KB
  budget. The symbol table is a quarter of a binary and nothing reads it at run
  time. Two other levers were tried and reverted with their numbers recorded in
  `Cargo.toml`, so the next person does not spend the afternoon rediscovering
  them: `panic = "abort"` saves 1 KB (the unwinding tables come from
  precompiled `std`), and `opt-level = "z"` on the runtime makes the binary
  *bigger* by blocking the inlining link-time optimisation needs.

### Added

- **Record a run, and replay it.** `GRASM_RECORD=j ./prog` writes a journal of
  everything the program read from outside itself; `GRASM_REPLAY=j` makes a
  later run take its answers from that journal and reach outside itself for
  **nothing** — not the reads and not the writes. Record a program that writes
  a file, delete the file, replay: identical output, and the file stays
  deleted. `print` and `eprint` are the deliberate exception, because a replay
  whose output you cannot see is not worth running.

  All **28** journaled builtins on **all four engines** — the whole `io`, `net`
  and `proc` surface. A recording made by the native build replays
  byte-identically under `grasm run` *and inside a wasm module that has no
  sockets, no processes and no filesystem*, TCP session and child process
  included. That is the case that matters: the failure happens in the fast
  engine and the debugger is in the other one.

  The effect system is what makes it cheap. Knowing which calls are
  non-deterministic is normally the hard part of building such a tool; here it
  was already written down, and `crates/grasm_sema/src/builtins.rs` says for
  each builtin what a replay owes it. A builtin with an effect row that says
  nothing about replay fails `cargo test`.

  Divergence is refused rather than guessed: a run that takes a different path
  from the recording stops with exit 7 and names the call that disagreed.

- **`grasm journal <file>`** prints a recording back out — every call in order,
  what it answered, and a tally of which builtins were reached. A journal is a
  file of framed bytes, and evidence nobody can examine is not evidence.
  `--full` stops truncating long payloads.

- **A range answers the list combinators** — `to_list`, `map`, `filter`, `sum`,
  `len` ([L-LIB-RANGE-OPS]). Building a list from a count needed
  `var xs: [i64] = []`, a `for`, a `push` and two closing braces; `(0..n).map(f)`
  says the same thing. It costs no syntactic form — a call is a call — so the
  kernel count in `spec/core.md` does not move. Most of the reason [G11] went
  from 1.41× Python to 1.22×, and 0.74× Go to 0.65×.

- **`spec/core.md` — the Grasm-Core / Grasm-Full split**, the commitment
  MASTERPLAN §19.2 makes binding and that four places in the repository asserted
  while none was true. One rule (a form is Full if and only if a
  meaning-preserving rewrite into other forms is written down), applied: 37 Core,
  18 Full, a rewrite beside every Full form. Thirty-seven is not thirty, and the
  document says so and names the seven forms that would close the gap.
  `cargo test -p grasm_spec --test core` reads the AST rather than a list, so a
  new construct that is neither kernel nor sugar fails the build; it also prints
  all three §19.2 budgets on every run.

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

At this release: **no S1, no S2 and no S3 are open.** One S4 (the UFCS help text
promises a call the compiler will not make).

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
