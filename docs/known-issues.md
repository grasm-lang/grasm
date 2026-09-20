# Known issues

Every defect that is known and not fixed, with its severity from
`docs/release-scope.md`, the clause it touches, and a reproducer small enough
to paste. S1 and S2 block a release; S3 and S4 do not, and this file is what
makes shipping them honest rather than quiet.

**Open S1: two. Open S2: none. Open S3: none. Open S4: none.**

## S1 — a value is released twice, in two programs

`packages/aes-demo` and `apps/broker` release a value that has already been
released. Natively they segfault or abort; under the interpreter both are
correct. Found by `scripts/usecheck.sh`, which builds the runtime with
`--features grasm_rt/poison` — a freed box is marked and never handed out
again, so the next thing to touch it says so:

    $ cargo build --release --workspace --features grasm_rt/poison
    $ ./target/release/grasm build packages/aes-demo/src/main.gr --native -o /tmp/a
    $ /tmp/a
      ok    the S-box starts 63 7c 77 7b
      ok    and ends 41 99 2d 0f b0 54 bb 16
    panic: use after free: dec on a value already released

Both are layout-sensitive, which is why every gate has been passing them. The
binary has to sit **directly in `/tmp`**: one directory deeper, or under
`$TMPDIR` — sixty characters of `/var/folders/...` on macOS — and the same
program runs clean. `difftest.sh` builds into a working directory under
`$TMPDIR`, so it has never seen either one; `leakcheck.sh` cannot see them at
all, because a premature free is not a leak. It counts what is still live at
exit, and a value freed too early is *less* live, not more.

### What is established, and one explanation already withdrawn

The defect is in code generation, and it is deterministic. The same function
compiles differently depending on what else is in the program:

    pub fn encrypt_block(round_keys: [i64], block: bytes) -> bytes {
        encrypt_with(Cipher{ round_keys: round_keys, table: sbox() }, block)
    }

The `Cipher` is handed to `encrypt_with`, which owns and releases it, so the
caller must not. Built as part of a program that only calls `encrypt_block`,
the caller emits **two** releases — one per parameter — which is right. Built
as part of the full demo it emits **three**: the struct as well, and that third
one frees what the callee already freed. Both counts are stable across repeated
builds to the same output path.

Why it crashes only sometimes is separate and is understood: the output path is
written into the binary, so `-o /tmp/a` and `-o /tmp/aaa` differ by the length
of that string and everything after it shifts. That changes the layout the
program runs with, which decides whether the freed box is reused before it is
read. It is not the bug; it is the weather.

**An explanation recorded here earlier was wrong and is withdrawn.** It said
the analysis tables — `field_calls`, `resolved_types`, `float_literals` and the
rest — collide because they are keyed by byte offset with no module. They are
keyed by offset, but the loader already shifts each file into a disjoint range
of one virtual span space (`parse_with_offset`, `base += len + 1`), so two
modules cannot share a key. The evidence for the claim was that padding
`packages/aes/src/main.gr` with forty lines of comment makes the program run
clean — which is true, and which only shows that changing the code changes the
layout, exactly as changing the output path does. That is the same mistake as
reading a benchmark without a control.

Also checked and ruled out: the compiler is deterministic. Two builds of this
program to the same path are byte-identical; three builds to *different* paths
differ in exactly one byte, which is the path.

So what is known is the codegen difference above, and that is where to look
next: what about compiling the rest of the demo makes the caller release an
argument whose ownership it has already handed over.

The release bar in `docs/release-scope.md` says S1 blocks a version. These two
do.

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
