# Compile speed, measured for the first time

`MASTERPLAN.md` §2.3 sets **G3: ≥500k LOC/s frontend, ≤50 ms incremental
edit-to-diagnostic**. `benchmarks/README.md` listed compile time under "Not
benchmarked" and it had never been measured once. This is that measurement,
taken 2026-10-07 on an 8-core Apple machine at load 3.3.

The programs are generated rather than taken from the corpus, because
throughput needs single compilation units of a known size: `scratchpad/cs/gen.py`
writes realistic Grasm -- a record type, a sum type, and three functions per
unit doing a `match`, a loop with a branch, and a struct literal with
interpolation -- rather than one line repeated. An equivalent Rust generator
writes the same shapes for comparison.

## The frontend is fine, and under target

`grasm check` is parse plus typecheck, which is what G3 names.

| lines | best of 5 | LOC/s |
|---:|---:|---:|
| 5,052 | 38 ms | 133,000 |
| 25,052 | 76 ms | 330,000 |
| 75,052 | 188 ms | 399,000 |
| 175,052 | 479 ms | 365,000 |

A linear fit over the four is **382,000 LOC/s marginal with 11.6 ms of fixed
startup**. Throughput climbs as that startup amortises and then flattens, so
the honest figure is **365-400k LOC/s against a 500k target: missed by about a
quarter**, and missed for an ordinary reason rather than a pathological one.

**Edit to diagnostic** is met for a file a person edits and missed for a large
one: 39 ms at 5,052 lines, **80 ms at 25,052** against a 50 ms target. The
crossover is around 15k lines in one unit.

## Fixed, 2026-10-07

One line was the whole of it. `compile_fn_as` opened with:

```rust
let user_names: HashSet<String> = fcx.cg.fns.keys()
    .filter_map(|k| k.rsplit("::").next().map(str::to_string))
    .collect();
```

built **once per function compiled**, walking **every function in the
program**, splitting each qualified name and allocating a `String` for it. At
9,000 functions that is 81 million allocations, and a profile of the 75,000-line
build put 76% of samples inside `compile_fn_as` with 1,668 in
`HashMap<String, _>::insert` and 1,762 in `rsplit`.

Only eighteen built-in method names can ever be the answer to the question it
was asking -- does the program declare a function that shadows a read-only
built-in, which [L-UFCS] would then let a method call reach. Those are now
recorded as functions are declared, and the question is a hash lookup.

| | before | after | |
|---|---:|---:|---|
| 25,052 lines | 1,549 ms | **621 ms** | 2.5x |
| 75,052 lines | 11,662 ms | **1,685 ms** | **6.9x** |
| 3x the lines costs | 7.5x the time | **2.7x** | linear |

Against `rustc -O` on the same shapes, Grasm is now **3.2x faster at 25k lines
and 2.5x faster at 75k**, where before it was 1.3x faster at 25k and 2.8x
*slower* at 75k.

Gates: 100 pass. The one failure, `gold`'s `30-async-chat`, reproduces
identically on the pre-change compiler (SIGSEGV, same three lines of output)
and is recorded in `docs/known-issues.md`.

## The defect, as it was found

| | Grasm | rustc `-O` |
|---|---:|---:|
| ~21-25k lines | 1,549 ms (16,173 LOC/s) | 2,013 ms (10,455 LOC/s) |
| ~63-75k lines | 11,662 ms (6,436 LOC/s) | 3,497 ms (18,029 LOC/s) |
| **3x the lines costs** | **7.5x the time** | **1.7x the time** |

At the size of one ordinary module Grasm's whole pipeline is **1.3x faster
than `rustc -O`** and level with `rustc -O0` (1,216 ms). Three times the code
and `rustc -O` is **2.8x faster than Grasm**. rustc is sublinear there, as it
should be -- fixed costs amortising -- and Grasm is steeply superlinear.

Two things it is not:

- **Not the incremental cache.** `--no-incremental` at 75k lines takes
  11,581 ms against the cache's 11,215: identical. (The cache does work at
  5k lines, where a rebuild is 269 ms against a 633 ms cold build, and stops
  paying above that -- which is the same defect seen from the other side.)
- **Not the frontend.** `check` over the same step is 76 -> 188 ms, 2.5x for
  3x the lines. Sublinear, like rustc's.

So the cost is in whole-program native code generation, and something there
scans more than it should as the program grows. Finding it is the next step,
and it is worth taking: it is the difference between a compiler that is faster
than `rustc -O` and one that is three times slower, on the same code.

## What this changes

Cranelift's design point is fast code generation, and the assumption here has
been that giving up LLVM's optimiser bought compile speed. It does -- 2.5x to
3.2x `rustc -O` on these shapes -- but that was invisible behind one quadratic
line, and nobody had looked because compile time had never been measured.

What is still open is **G3 itself**: the frontend at 365-400k LOC/s against
500k, and edit-to-diagnostic at 80 ms for a 25,000-line unit against 50 ms.
Both are frontend numbers and neither is touched by this fix.

Reproduce with `scratchpad/cs/gen.py N` and `genrs.py N`, timing
`grasm check`, `grasm build --native` and `rustc -O` on the output.


## The frontend, profiled (2026-10-07)

G3's frontend half is still short: 365-400k LOC/s against 500k, and mildly
superlinear -- 625,052 lines run at **286k LOC/s** where 175,052 run at 365k.

A profile of `check` over 625,052 lines, by share of the 1,502 samples under
`check_program`:

| | samples | |
|---|---:|---:|
| `check_module` -- typing the bodies | 733 | 49% |
| the collection passes before it | ~450 | 30% |
| `rows_and_closures` -- the effect fixpoint | 123 | 8% |
| everything else | ~200 | 13% |

`check_modules_in` is one function holding a sequence of `for f in files`
passes -- `collect_linear`, `collect_types`, `collect_effects`, then aliases,
then the bodies. The profiler reports it four times because those passes
inline separately, not because it runs four times.

So the frontend is **not** the backend's story. There is no single quadratic
line standing in front of half the time; the cost is spread across a fixpoint,
four collection passes and the body check, and the superlinearity is worth
about 22% over a 3.5x size increase rather than the 7.5x the backend showed.

Reaching 500k LOC/s therefore means either removing the superlinearity *and*
finding roughly a quarter more, or running the collection passes in parallel
across modules -- which does nothing for a single large file and would be
worth measuring on the 651-package corpus instead, where it is many modules
rather than one.

That is a project, not a fix, and it is recorded here rather than started.

## Did the `no_std` and scheduler work cost compile speed? (2026-10-12)

No. `panic = "abort"` now applies to the compiler too, so this was worth
checking rather than assuming.

Run sequentially, master then branch at each size, the branch looked 3-9%
slower in **all four** rows -- consistent enough to look real. Run
**alternating**, best of six each:

| lines | master | branch | ratio |
|---:|---:|---:|---:|
| 25,061 | 87 ms | 88 ms | 1.012x |
| 175,056 | 721 ms | 704 ms | 0.977x |

Opposite directions, both inside 2.3%. The sequential figure was an ordering
artefact, which is the whole of [[bench-interleave-ab]]: this machine drifts
more between consecutive runs than the change being measured.

**The absolute figure here is not comparable to the table above.** These
programs are generated by a different generator -- `type Rec` with three
fields, two functions per record, interpolation of three fields each -- and
give 240-290k LOC/s where the original gave 365-400k. More work per line, not
a regression: the two numbers measure different programs. G3's standing is
unchanged at the recorded 365-400k against a 500k target, and nothing in this
branch moved it either way.
