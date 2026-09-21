# Known issues

Every defect that is known and not fixed, with its severity from
`docs/release-scope.md`, the clause it touches, and a reproducer small enough
to paste. S1 and S2 block a release; S3 and S4 do not, and this file is what
makes shipping them honest rather than quiet.

**Open S1: none. Open S2: none. Open S3: one. Open S4: one.**

Closed on 2026-09-21: `fuzz_exits` disagreed on four programs in eighty-five,
and each was the same leak. A tuple literal of two or three elements is built
in one call, and the guard on that path asked `body_can_fail` — whether an
element could `fail`. An element can also leave by `return`, `break`,
`continue` or `try`, and when it did, the elements already built were
abandoned with nowhere to release them from. `elem_can_leave` asks the whole
question now, and T-TUPLE carries all four spellings; it exits 7 under
`GRASM_LEAK_CHECK=1` on the compiler before the fix.

## S3 — `gitsigndiff` differs on eight cases in a hundred and five

`scripts/gitsign_diff.sh` reports `agreeing: 97   differing: 8`, and several
of the differing cases produce the *same* wrong hash, which points at one
cause rather than eight. Also present on `bcf035d`, so it predates this
session.

## S4 — `grasm_sema`'s deep-chain tests overflow the stack in a debug build

`cargo test -p grasm_sema --test check_tests` aborts with `has overflowed its
stack` on `a_chain_of_field_reads_is_linear`, which checks a thousand-deep
`n.next.next...` projection. It is flaky rather than constant: the same test
binary passed earlier the same day and failed later, and `RUST_MIN_STACK=32M`
made it pass once and then moved the abort to a different, unnamed thread at
64M. Release builds are unaffected, so every gate in `scripts/allgates.sh`
passes; only the debug unit-test run sees it.

The checker recurses once per projection and a debug frame is large, so a
thousand of them sit near the limit and whether they fit depends on what else
the process has done. Not a wrong answer — an abort — and not reachable from a
compiled program, which is why it is S4. The fix is for the checker to walk a
projection chain iteratively, or for the test to run its body on a thread with
a stack it asks for; shrinking the chain would delete the thing the test is
for.

Closed on 2026-09-20, and the entry it replaces was wrong twice before it was
right. Two programs — `packages/aes-demo` and `apps/broker` — released a value
twice and segfaulted natively while running correctly under the interpreter.
Neither was a code-generation defect. Both were **objects compiled by an
earlier build of the compiler and reused by a later one**.

`build_native_incremental` caches one object per module in `.grasm-cache`
beside the output, and recompiles a module only when its key is missing. The
key was the module's name, its source, and the program's whole signature
surface — and nothing about the compiler doing the compiling. So a session
spent changing code generation and rebuilding left every cached object valid,
and each new binary linked yesterday's code.

Everything that made it look like a program bug follows from that:

* *The output path mattered.* The cache lives beside the output, so `-o /tmp/a`
  and `-o /tmp/sub/a` are different caches with different staleness.
* *Padding the source fixed it.* It changes the key, so the module recompiles.
* *The same function emitted two releases in one program and three in another.*
  One of those was read from a stale object.
* *`difftest` never saw it.* It builds into a fresh directory per run, so its
  cache is always cold — which is also why the gate was right and the bug was
  real at the same time.
* *Instrumenting the backend printed nothing.* Nothing was being compiled.

The fix is `compiler_fingerprint()` in the cache key: the version, and the
executable's size and modification time, which cost one `stat` and change for
a user who upgrades as surely as for a developer who rebuilds. With it, a
compiler rebuild invalidates the cache; without it, a released upgrade would
silently mix old objects into new binaries. With a cold cache both programs run
clean, `usecheck` reports no use-after-free, and the previously failing
difftest slice is 71 of 71.

`scripts/usecheck.sh` and the `poison` feature stay. They are what turned an
intermittent segfault into a named, repeatable `use after free`, and the gate
`leakcheck` cannot replace.

F92 was open here and is fixed: a loop body is a block in statement position,
so its last expression is handed to nobody, and neither `while` nor `for`
released it.  See `ProjectGold/FINDINGS.md`.

Closed on 2026-08-17, all four found by writing a calculator in the language
rather than by a test: a closure in a `match` arm could not see the arm's own
pattern binding (S3 — a correct program had no spelling); the `--native` refusal
for a closure inside `Some(...)` in an arm stated a false reason and offered a
fix the source already contained; `&&` and `||` were reported as "unexpected
character `&`" with no mention of `and` and `or`; and a range's "no method" help
proposed `fn count(range, ...)`, a signature nobody can write.

Closed on 2026-08-16: the effect-row fallback pool (and the mechanism behind
it), writes through a `ref` under `--native`, returning a closure under
`--native`, and the UFCS help text.

This file is not empty because nothing is wrong — it is empty because what was
wrong is listed above as closed. The next round of evaluation is what refills
it.

## Nothing open

The six defects that were open on 2026-08-17 are closed:

* an incremental `--native` rebuild shipped a **stale imported constant** — the
  cache keyed a module's object on its own source and on the program's
  *signature* surface, and a signature omits a constant's value. Now keyed on a
  fingerprint of everything the object read, which is the rule rustc's
  incremental engine uses.
* an **effect operation's result** was not row-checked, so a handler could hand
  a printing function through an empty-row result and a `capabilities = []`
  package reached `io`. `check_expr` returned early for a closure after
  checking the closure's own row.
* the **registry** was found only when the working directory was exactly the one
  holding `.grasm/` — the walk up was by `parent()` from a relative path, which
  has no parents.
* [L-LIB-SORT-BY] said a sort key "must be `i64`, `f64` or `str`" while tuple
  keys worked; [T-REF-WRITE] said the compiled backend refuses writes through a
  reference, a day after it stopped doing so.
* a **deeply nested tuple literal** cost O(n³) to check — 1000 levels took 7.8
  seconds, now 31 ms.

## S3 — the checker did not enforce [T-GENERIC-TYPE] — *closed*

`let r = Good(1)` for `type Res[T, E]` passed `grasm check`, ran under `grasm
run` and was refused by `--native`. The clause said the annotation was the
language's rule and one engine of three enforced it.

Closed on 2026-08-25 by teaching the checker, which is what the clause asked
for. The check waits for inference to finish, so a parameter a *later* line
fixes is still determined: `let a = Good(1)` followed by a call wanting
`Res[i64, str]` compiles everywhere. The backend reads the checker's answer
rather than inferring from the constructor's arguments alone, which is what it
had been doing and why it refused a program the checker had settled.

## S3 — over-strict, effect-row inference

### The fallback pool — *removed*

[T-FN-TYPE-ROW-CARRIED] (`experimental`). Closed on 2026-08-16, and then the
mechanism behind it was deleted. The type checker publishes the row of every
function-valued expression it types and of every call site whose callee is not
an expression of its own; the effect pass reads that. The syntactic
reconstruction — a map of name to sources, a pool of every function value a
body mentions, the rows a signature writes at nameless positions, and the
per-binder special cases that fed them — is gone, 511 lines of it.

There is no fallback pool to be over-strict any more, so the shape recorded
here (a `match` payload charged a sibling parameter's row) cannot recur.

### `--native` — *no gaps in the repository's programs*

Every program in `spec/conformance/`, `examples/`, `projects/` and
`benchmarks/` compiles natively: **223 identical across all three engines, 0
unsupported, 0 leaking.**

The §11 table listed seventeen shapes the backend refused and now lists two,
neither of which is a defect: a generic sum constructor that leaves a type
parameter undetermined, which is the language's rule ([T-GENERIC-TYPE]) rather
than one engine's, and polymorphic recursion, which is what monomorphization
is. `scripts/nativegaps.sh` proves the list matches the compiler in both
directions, over 55 cases, comparing output, exit code and leaks.

Closed on 2026-08-16: writes through a `ref` (including around a cycle) and
returning a closure. Closed on 2026-08-24 and 25: the prelude's `IoError`
variants, a range held as a value, `map` with a function returning nothing,
`==` on two bare `ref`s, a call through a non-name callee, a `fold` seeded with
an unannotated `map()`, a closure whose parameter types nothing local fixes, a
type parameter only in a function result, every way of using an effect handler
as a value, a UFCS call of a generic free function, generic `type` declarations
for both structs and sums, and `fail` inside a fallible closure passed to `map`. Closed on 2026-08-25:
interpolating a unit whose type was inferred rather than annotated (F45), and
`return` of an `if` whose every branch fails (F46) — the second a Cranelift
verifier failure, so a defect rather than a gap. Closed on 2026-08-26: a write
through a `ref` to a sum, which [T-REF-VALUE] always described and the
implementation excluded along with the structs (F48), and a handler operation
capturing a unit, which panicked inside Cranelift rather than refusing
anything (F50), and a closure declared inside another closure, which was
refused for every representation and had never been written down as a gap
(F51), and an empty `map()` as an element of a list literal, which was the
one position that asked the probe rather than the lowering (F52), and a range reaching a method from any shape that produces one rather
than only from a name -- which also gave the range type the spelling `range`,
so one can be a parameter, a result or a field (F53); and `fold`,
`unwrap_or`, `expect` and `parse_int_base` where the backend has to name a
call's result type rather than a receiver's (F54). And a write *inside* a referent, `r.value[0] = v`, refused
at two sites one level apart while reading the same place always compiled
(F56). Closed on 2026-08-27: a handler left installed when a `fail` or
`return` leaves its `with` block, which answered from the wrong handler and
then from a dead frame (F59), and the `return` itself, whose following
statement was emitted past the terminator (F60), and the same for `break`
and `continue`, which leave the block without leaving the function (F61); and the handler's
captures, which every one of those four exits kept (F62); and a `region`, a
plain block or an `unsafe` block that was left early, whose following
statement was emitted past the terminator (F63); and a `parallel` block's
task list, kept by every early exit, whose statement had the same fault
(F64); and a list, map, tuple or struct literal under construction,
abandoned when an element or field left the function (F65); and reading
`.value` through a captured `ref`, which [T-REF-CAPTURE] is the clause for
(F67).

## S4 — diagnostics

### `x.f()` does not reach a library free function — *closed*

Closed on 2026-08-16 by narrowing the clause rather than widening the
implementation. [L-UFCS] now says the language's own free functions are not
methods of any type, and `"hi".print()` is refused with help that names the
spelling that works — `"hi" |> print`, which was always there and is charged
the same row a call is. The old help offered UFCS for `print`, which is the
call it had just refused.
