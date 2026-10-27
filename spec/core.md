# Grasm-Core — the kernel language

MASTERPLAN §19.2 makes one commitment binding on this project:

> **Grasm-Core:** ≤ ~30 constructs. Formal spec ≤ 50 pages. Reference interpreter
> ≤ 5,000 lines, implementable by one competent engineer in ~3 months (the
> Lua/Scheme/WASM property).
>
> **Grasm-Full:** everything users see beyond core — async/task scopes, actors,
> channels, derives, string interpolation, comprehensions, contracts sugar —
> defined as self-hosted libraries and macros that **provably lower to Core**.

The reason it is binding is the fourth consequence listed there: *"The full Core
spec + semantics fits in one LLM context window. No mainstream language can be
completely known by an AI at inference time; Grasm-Core can."* That is the one
claim in this project no competitor can copy quickly, and it is worth nothing
unless the split is real, mechanical, and checked.

Until this document existed the split was **asserted and not true**. `spec/README.md`
said "this directory is the normative description of Grasm-Core"; the AST had 58
syntactic forms, the reference interpreter 9046 lines, and no rewrite from any
construct to any other was written down anywhere. This document defines the
split, and `crates/grasm_spec/tests/core.rs` fails if it drifts.

## The rule

> A syntactic form belongs to **Full** if and only if a meaning-preserving
> rewrite into other forms exists and is written down here. Everything else is
> **Core**.

Nothing is Full because it is rarely used, and nothing is Core because it is
convenient. The test is the rewrite.

Two consequences worth stating, because both were tempting and both are wrong:

* A form is not Full merely because a *library function* could do the same job.
  `xs.map(f)` is a call to a function, so `map` is not a construct at all; but
  `for x in xs { … }` is a form, and it is Full because a rewrite into `while`
  exists.
* A form is not Core merely because the implementation happens to handle it
  directly. Every Full form below is handled directly by the current
  implementation for speed. Lowering is about what the language *is*, and an
  implementation may take shortcuts as long as they agree with the rewrite —
  which is what `scripts/difftest.sh` already checks for the whole corpus.

## Where the split falls today

**Core: 37 forms. Full: 22 forms.**

Thirty-seven is not thirty. §19.2 says "≤ ~30", and this is over it. The gap is
recorded rather than closed by re-counting: §"What would have to move" below
names the seven forms that would take Core to 30 and what each would cost. A
number that is met by redefining what counts is not a number.

The literal forms — `1`, `1.0`, `true`, `"s"` — are counted as **one** construct
("a literal"), because they are one syntactic category with four token kinds.
That is the only collapse taken, and it is stated so the count can be audited.

## Core

### Expressions (14)

| # | Form | Why it cannot be rewritten |
|---|---|---|
| 1 | literal (`1`, `1.0`, `true`, `"s"`) | ground values |
| 2 | `x` (name) | reference to a binding |
| 3 | `f(a, b)` | application |
| 4 | `e.f` | product projection |
| 5 | `e[i]` | a *place*, not a call — it appears on the left of `=` |
| 6 | `a + b` | primitive operation |
| 7 | `(a, b)` | product introduction |
| 8 | `T{ f: e }` | nominal product introduction |
| 9 | `if c { … } else { … }` | branching |
| 10 | `match e { … }` | ADT elimination |
| 11 | `{ … }` | sequencing and scope |
| 12 | a closure literal, `x` to `e` | abstraction |
| 13 | `region { … }` | tiered ownership: a scope with an allocation discipline |
| 14 | `unsafe { … }` | the capability gate; nothing else can express it |

### Statements (9)

| # | Form | Why |
|---|---|---|
| 15 | `let x = e` / `var x = e` | binding |
| 16 | `x = e` | assignment to a place |
| 17 | `e` | expression statement |
| 18 | `return e` | non-local exit |
| 19 | `while c { … }` | iteration |
| 20 | `break` | loop exit |
| 21 | `continue` | loop restart |
| 22 | `with E handled_by h { … }` | effect handler installation |
| 23 | `concurrent { … go { … } … }` | tasks that take turns; no sequential form has a turn to give ([E-SCHEDULE]) |

### Declarations (5)

| # | Form | Why |
|---|---|---|
| 24 | `fn f(…) -> T / {…} { … }` | function definition |
| 25 | `type T = …` | ADT and alias definition |
| 26 | `effect E { … }` | effect declaration |
| 27 | `extern fn f(…)` | the C boundary; no Grasm form denotes it |
| 28 | `import m` | module reference |

### Types (6)

| # | Form | Why |
|---|---|---|
| 29 | `T`, `T[A]` | named type, applied |
| 30 | `fn(A) -> B / {E}` | function type, carrying its row |
| 31 | `T ! E` | error union |
| 32 | `ref T` | generational reference |
| 33 | `(A, B)` | product type |
| 34 | `(A or B)` written with a bar | error-union member set |

### Patterns (3)

| # | Form | Why |
|---|---|---|
| 35 | `_` | ignore |
| 36 | `x` | bind |
| 37 | `V(a, b)` | ADT elimination |

## Full — and the rewrite for each

Each row is a rewrite that must preserve meaning. Where a rewrite introduces a
name, that name is fresh and cannot capture (`§ fresh`).

### Expressions (6)

| Form | Rewrite into Core |
|---|---|
| `[a, b, c]` | `List.of(a, b, c)` |
| `{k: v, …}` | `Map.of(k, v, …)` |
| `x.f(a)` | `f(x, a)` — the same call, by [L-UFCS]; where `f` is a field of `x`'s type, `(x.f)(a)`, by [T-FIELD-CALL] |
| `-e` | `0 - e` |
| `!e` | `e == false` |
| `a..b` | `Range.new(a, b)` |
| `try e` | `match e { Ok(v) -> v  Err(er) -> return Err(er) }` — verified by running both spellings against each other, on the ok branch and the failing one |

String **interpolation** — `"a{e}b"` — is Full within form 1: it rewrites to
`"a" + to_str(e) + "b"`. The literal itself is Core.

### Statements (4)

| Form | Rewrite into Core |
|---|---|
| `let (a, b) = e` | `let § = e` then `let a = §.0` and `let b = §.1` |
| `for x in xs { B }` | `let § = iter(xs)` then `while has_next(§) { let x = next(§); B }` |
| `parallel { … }` | `{ … }` with each `spawn` body run in order — [E-NOT-PARALLEL] says tasks behave as if run in order, so the rewrite is exact |
| `spawn { B }` | the block `B`, deferred to the end of the enclosing `parallel` |

### Declarations (9)

| Form | Rewrite into Core |
|---|---|
| `let NAME = e` at module level | `fn NAME() -> T { e }` plus a use-site rewrite of `NAME` to `NAME()`; sound because [E-CONST] already fixes one value for the whole program |
| `test "name" { B }` | `fn §test_name() { B }`, collected by the runner rather than by the language |
| `bench "name" { B }` | `fn §bench_name() { B }`, likewise |
| `interface I { … }` | nothing: a constraint the checker reads, erased before any engine runs ([T-INTERFACE]) |
| `impl I for T { fn m(self, …) … }` | `fn m@T(self: T, …) …`, and each `x.m(…)` the checker resolved through it becomes `m@T(x, …)` by `x`'s type |
| `actor A(ps) { var f: T = e … fn m(self, …) … }` | `type A@state = { f: T … }`, `type A = { §state: ref A@state }`, `fn A(ps) -> A` building one, and each method as an `impl` method `m@A` with `self.f` read as `self.§state.f` ([E-ACTOR]) |
| `thread_local C: T` | nothing at the declaration; each `mem_tls(C)` is a call of the primitive that keeps one zeroed `T` per thread per name, `mem_tls_cell("m::C", mem_size_of(T))` ([T-TLS]) |
| `process_local C: T` | likewise, one zeroed `T` per process, `mem_process_cell("m::C", mem_size_of(T))`; a runtime package's alone ([T-RUNTIME]) |
| `requires` and `ensures` clauses on `fn f` | `f@body` holds the body, each clause is a `bool` function, and `f` checks them around a call to `f@body` ([T-CONTRACT-REQUIRES]) |

### Types (1)

| Form | Rewrite into Core |
|---|---|
| `[T]` | `List[T]` |

### Patterns (5)

| Form | Rewrite into Core |
|---|---|
| `1` / `true` / `"s"` in a pattern | bind a fresh name and guard: `§ if § == 1` |
| `(a, b)` | nested projection: bind `§`, then `let a = §.0`, `let b = §.1` |
| an or-pattern (`p` alternated with `q`) | two arms with the same body |

## What would have to move, to reach 30

Seven forms, in the order they are least defensible as Core. The number in
parentheses is the **net** saving — what a change removes minus what it adds
back — because that is the only figure the arithmetic below can use. The first
entry is the reason to say so: it takes two forms out and puts one in.

1. **`break` and `continue`** (net 1). Rewritable if `while` carried a result and
   bodies could yield — a labelled-block form. That is one construct in and two
   out, a net saving of one, and it makes every loop harder to read. Not worth
   it for the count alone.
2. **`region`** (1). It is a scope with an allocation discipline; a rewrite
   needs the discipline to be expressible some other way, which today it is not.
   Moving it means giving Core an allocator interface — a real design change.
3. **`unsafe`** (1). Only removable if the capability gate becomes a type rather
   than a form. That is the [T-CAPABILITY] direction and would be a genuine
   simplification.
4. **`import`** (1). Removable if modules become values. That is a large change
   with a large payoff and is the honest candidate.
5. **`(A | B)` error-union member sets** (1). Removable if unions are ordinary
   ADTs, which costs inference precision at `try`.
6. **`e[i]`** (1). Removable if places are first-class, which is a bigger idea
   than the saving. It is *not* removable by giving lists an `at` and a
   `set_at` and calling indexing sugar for them: nobody would write those
   methods, and inventing surface so a count comes out right is the thing this
   document exists to prevent.
7. **`concurrent` and `go`** (net 1). Removable if a task switch were an
   effect a scheduler handles, as MASTERPLAN §8.6 wants channels built on
   effects: `concurrent` would be a `with` over it. Today a handler cannot
   hold a suspended computation, so this needs continuations in Core first.

Five of those seven are real design work, not bookkeeping. The count is 37 and the
route to 30 is written down; claiming 30 today would be false.

## What the gate checks

`cargo test -p grasm_spec --test core` fails when:

1. a syntactic form exists in the AST and is classified in neither table here;
2. a form is classified in both;
3. a Full form has no rewrite written beside it;
4. the Core count printed here disagrees with the Core table;
5. the reference-interpreter and specification budgets are exceeded by more than
   the margin recorded below.

The budgets, measured rather than asserted:

| Budget | §19.2 target | Today |
|---|---|---|
| Core constructs | ≤ 30 | **37** |
| Reference interpreter | ≤ 5,000 lines | **9046** |
| Specification | ≤ 50 pages | **~70 pages** |

None of the three is met, and all three are measured on every run. The
specification's is the binding one: a new clause is paid for out of prose.

How each figure got here, and what closing the gap would take, is history
rather than specification: it is in `docs/known-issues.md`.
