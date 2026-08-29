# Removing the backend's second type inference

A working log. Written as it goes, including the tracks that turned out to be
wrong, because a log that records only the route that worked is a log that
teaches nothing about the ones that did not.

## Why

Nine of the last twelve findings trace to one fact:

> The checker infers the type of every expression. It does not hand those types
> to the backend. So the backend reimplements type inference — about 950 lines
> with 49 distinct refusal sites — and every place the two disagree is a defect.

The clusters, in the findings' own words:

| cluster | instances |
|---|---|
| "the probe knew less than the lowering" | F48, F52, F53, F67, F74, F75 |
| "the backend re-derived a rule the checker already knew" | F38, F39, F71 |

The evidence is `Analysis`, the struct the checker *does* hand over. Twelve
fields, ten of them span-keyed patches — `float_literals`, `ok_wrapped`,
`map_get_value_kind`, `value_rows`, `callee_rows`, `field_calls`,
`fallible_closures`, `closure_param_types`, `generic_ctor_types`,
`const_types`. **Each was added after a defect**, each supplying one fact about
certain expressions that the backend could not work out for itself. Ten patches
to a hole with one shape.

## The precedent

This exact disease was cured once here, for *effects* rather than types. From
`docs/release-scope.md`:

> it was a syntactic approximation of a type-and-effect system rather than the
> thing itself. That approximation was removed on 2026-08-16. Rows now travel
> through the type checker's own unification, and the effect pass reads the
> checker's answer instead of reconstructing it: 511 lines of guessing deleted,
> every gate green, and the class of defect that produced most of the last four
> rounds' findings no longer has a mechanism.

Effects were cured. Types were not.

## The plan

Incremental, so the gates are green at every step and coverage is measurable.

1. **Record.** The checker already has `expr_types: SpanMap<Type>`, but it is a
   memoisation *cache* — only "cacheable" expressions go in. Record every
   inferred expression instead.
2. **Export.** Put the map in `Analysis`.
3. **Consult.** `probe_type` looks the span up first and falls back to its
   existing logic. Nothing is deleted yet.
4. **Measure.** Count how often the lookup answers and how often the fallback
   runs, over the whole corpus and all 2,177 generated programs.
5. **Delete.** Remove the arms the fallback no longer reaches, and the
   `Analysis` patches the map makes redundant.

Step 3 is the safety property: at no point is there a window where the backend
has neither answer.

## Log

### 2026-08-28 — step 1 begins

Starting state, measured:

| | |
|---|---|
| `probe_type` | lines 10670–11619 of `crates/grasm_native/src/lib.rs` |
| refusal sites inside it | 49 |
| `Analysis` fields | 12, of which 10 are span-keyed patches |
| gates | all green, 261 corpus programs, 2,177 generated |
| open defects | none |

### 2026-08-28 — steps 1 to 3, and a regression they caused

**Step 1, record.** `Checker::resolved_types`, written for every expression
whether or not `expr_types` would cache it. A span visited twice is recorded
twice and the later answer wins: the second visit knew more.

**Step 2, export.** `Analysis::resolved_types`, rendered with `c.show` rather
than handed over as a `Type`. Rendering is the interface `generic_ctor_types`
already uses, and it keeps the backend with *one* function that turns a written
type into its own — handing over a `Type` would need a second converter, which
is the shape of the problem being removed. A type still holding a unification
variable renders with `_` and is left out: an answer the checker did not settle
is not an answer.

**Step 3, consult.** `probe_type` looks the span up and falls back to its own
reasoning. Nothing deleted. `probe_hits` and `probe_misses` count which ran.

**And then four gates went red.**

`examples/function_values.gr` compiled and took SIGSEGV. Minimal case, six
lines:

    fn square(n: i64) -> i64 { n * n }
    fn main() {
        let ns = [1, 2, 3]
        print("{ns.par_map(square)}")
    }

Confirmed mine by stashing the change: exit 0 before, 139 after.

**The cause — and it is worth writing down, because it is an assumption I did
not know I was making.** The backend rewrites some calls before compiling them.
`xs.par_map(square)` is lowered as `xs.par_map(|__par_arg| square(__par_arg))`,
and the synthesised call carries `square`'s own span. The checker settled that
span as the type of the function *value*; the node standing there after the
rewrite is a *call*, whose type is the return. Reading one as the other read a
machine word as a pointer.

**Spans are unique per source expression. They are not unique after
desugaring.** Step 3 assumed the first and needed the second.

**The fix, and the wrong track it avoided.** The obvious repair is a flag on
the function context, set around each desugared body. There are six call sites
of `desugar_fn_arg`. A caller that forgot to set it would reintroduce the
fault, silently, for that one method — which is *precisely* the defect shape
this whole change exists to remove, and it would have been added by the removal
itself.

So the record is kept at the one place that synthesises. `desugar_fn_arg`
inserts the span it reused into `Codegen::synthesised_spans`; `probe_type`
skips those. One choke point, no per-site discipline.

State after: `function_values` agrees on three engines, `nativegaps` matches
the compiler, `par_map(named)` exits 0.

**What this says about the plan.** Step 3's safety property held — the fallback
was there, so the failure was a wrong answer from the *new* path rather than an
absence. But it also showed the plan was missing a step: the map's key has an
invariant (one span, one expression) that the backend violates on purpose, and
nothing checked it. Step 4 must measure that, not only the hit rate.

### 2026-08-28 — step 4, measured

`GRASM_PROBE_STATS=1` counts which answer was used; `GRASM_PROBE_MISS=1` names
the shape of each expression that still falls through. Over all 231 whole
programs in the corpus, compiled natively:

| | |
|---|---|
| type questions asked | 1,001 |
| the checker answered | **947 (94%)** |
| the backend inferred for itself | 54 (6%) |

The 54, by expression shape:

| shape | misses | why, so far |
|---|---|---|
| `ident` | 41 | a name the checker did not settle, or one a rewrite synthesised |
| `list` | 25 | an empty literal whose element type only the position gives |
| `call` | 10 | includes the synthesised calls, which are skipped on purpose |
| `tuple` | 5 | |
| `int` | 4 | a literal in an `f64` position, already patched separately |
| `if` | 3 | |
| `field` | 3 | |
| `map` | 1 | an empty `map()` |

**What this says.** 94% on the first attempt, with no arm deleted and no
behaviour changed, is the number that says the approach is sound: the checker
really does know almost everything the backend was re-deriving.

The remaining 6% is not one problem. It is at least three:

1. **Spans a rewrite reused** — skipped deliberately, and correctly. These will
   never be answered by the map and the fallback is the right answer for them.
2. **Expressions the checker never visited.** An empty `[]` in a position the
   checker resolved by unification may have no recorded type of its own.
3. **Types that did not settle** — rendered with `_` and dropped on export.

Only the second and third are worth closing. The first is the design.

**Not deleting anything yet.** An arm reached zero times over this corpus is
not an arm reached zero times over every program — the corpus is 231 programs,
not a proof. The next measurement is the same counters over the 2,177
*generated* programs, which explore shapes the corpus does not contain. That is
what would justify a deletion.

### 2026-08-28 — all gates green with steps 1 to 4 in

| gate | |
|---|---|
| difftest | 261 identical, 0 mismatched, 0 unsupported |
| leakcheck | 261 leak-free |
| crosstest | 261 identical |
| **wasmtest** | **224 identical**, 37 skipped — was 189/72 |
| agreetest, nativegaps, replaytest, gatecheck, northstar, gold, findings, clippy, unittests | pass |
| 15 generators | 2,177 programs, 0 disagreed |

The wasm number moved for a different reason, recorded here because it was
found while measuring this: 38 of that gate's 72 skips were multi-module
programs skipped as "wasm32 has no filesystem", which is true of the *disk
loader* and not of the engine — a source bundle loads through the same
function the disk uses, and `grasm build --source` writes one. Bundling them
instead of skipping surfaced F81, in which `--source` reported success and
wrote an artifact that could not be loaded.

**Not deleted yet, and the reason stands.** 94% over 231 programs is a sample.
The next measurement is the same counters over the 2,177 generated programs,
which reach shapes the corpus does not contain.

### 2026-08-29 — step 5, measured over the generated programs

The corpus is 231 programs somebody wrote. The generators build 2,177 that
nobody wrote, and they reach shapes the corpus does not contain — which is
exactly why a deletion needs their number and not only the corpus's.

Measured by wrapping the compiler in a shim that tallies the counters, so the
generators see nothing different (they capture the compiler's stderr and
surface it only on failure, so the counters could not be read from their
output):

| | corpus | generated |
|---|---|---|
| builds | 231 | 2,193 |
| checker answered | 947 | 78,911 |
| backend inferred | 54 | 11,891 |
| **coverage** | **94%** | **86.9%** |

The misses, by shape, over a sample of five generators:

| shape | misses | share |
|---|---|---|
| `list` | 3,632 | 59% |
| `closure` | 2,282 | 37% |
| `call` | 107 | 2% |
| `ident` | 86 | 1% |
| `int` | 2 | — |

**Two shapes are 96% of it, and they are the same shape twice.** A list literal
and a closure literal both get their type from the *position they fill* rather
than from their own text: `[]` has no element type of its own, and `|n| n * 2`
has no parameter type of its own. The checker settles both by unification, and
what this export records at the moment of the visit is a type that may still
hold a variable — which renders with `_` and is dropped, deliberately, because
an answer the checker did not settle is not an answer.

An ordinary program shows none of this: a file with an annotated empty list, an
empty `map()`, a bound closure, an inline closure argument and a nested empty
list misses **zero** times. The misses live in the shapes generators build and
people rarely write.

**Deletion is not justified, and this is where that is decided.** The fallback
still ran 11,891 times. An arm the corpus never reaches is not an arm nothing
reaches, which was the reason for this measurement, and it answered no.

**What was achieved anyway, and it is the part that mattered.** The goal was
not to delete 950 lines — it was to stop two implementations of type inference
disagreeing. Where the checker answers, they *cannot* disagree, because there
is only one answer. That is 94% of real programs and 87% of generated ones, and
it is a permanent reduction in the defect class rather than a cleanup.

**What would close the rest**, if it is worth doing later: record each
expression's type *after* the enclosing function's unification settles, rather
than at the moment of the visit. That would resolve the positional cases and
leave only the spans a rewrite reused — which are correctly skipped and always
will be.

## Where this stands

Steps 1 to 5 done. Nothing deleted, on purpose. The class of defect that
produced nine of the last twelve findings has lost its mechanism wherever the
checker answers, and the measurement says where that is.

