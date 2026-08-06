# Benchmarks

Six programs, each written the way somebody would naturally write it in that
language — no hand-vectorized C competing against idiomatic Python, and no
Grasm program tuned in ways the others were not.

Run them with `./scripts/bench.sh`. Every implementation prints the same
checksum, and the harness refuses to report a timing when the checksums
disagree: an optimizer that deletes the work being measured is the classic way
a benchmark lies, and a wrong answer is not a fast answer.

It also refuses to run at all on a busy machine. A benchmark taken under load
does not just come out slower, it comes out slower *unevenly*: absolute times
inflate, and the multi-core rows collapse, because the speedup being measured
is competing with whatever else wants the cores. Re-measuring `mandelbrot` at
load average 5 on 8 cores gave 2.0x where an idle machine gives 5.9x, and C —
which nobody had touched — moved from 36ms to 58ms in the same run. That reads
as a regression and is not one, so `BENCH_IGNORE_LOAD=1` is required to record
numbers taken that way, and they should not be published.

## What each program measures

| Program    | Stresses                                       |
|------------|------------------------------------------------|
| `fib`      | function call overhead and integer arithmetic   |
| `sieve`    | list indexing and tight loops                   |
| `matmul`   | nested indexing over a flat list                |
| `wordfreq` | hashing, string handling, map updates           |
| `sort`     | comparison-heavy work over a large list         |
| `parmap`   | multi-core throughput, each language's idiom    |
| `mandelbrot` | a pure float kernel — the workload "N times faster than Python" claims come from |

## How to read the numbers

Grasm has two backends. `grasm run` is the tree-walking reference interpreter
and is expected to be slow — it exists to define semantics, not to be fast.
`grasm build --native` compiles through Cranelift and is what should be
compared against the others.

C, Rust, and Go are compiled with ordinary release settings (`-O2`,
`--release`/`-O`, default `go build`). Python is CPython. Nothing is run with
profile-guided optimization or LTO, on any side.

These are small programs. They say something about call overhead, loop
codegen, and allocation, and nothing at all about how a large program behaves.

## Results

Apple M-series, macOS 25.1, best of 5 runs, wall clock in milliseconds. Lower
is better. Reproduce with `./scripts/bench.sh`.

| program  |   C |  Rust |  Go | grasm-native | grasm-interp | Python |
|----------|----:|------:|----:|-------------:|-------------:|-------:|
| fib      |  25 |    26 |  28 |       **27** |         1206 |    153 |
| sieve    |  28 |    28 |  29 |           39 |         2229 |    370 |
| matmul   |  25 |    26 |  29 |           32 |         5795 |    974 |
| wordfreq |  54 |    48 |  39 |          117 |          472 |    153 |
| sort     |  58 |    33 |  70 |       **47** |          323 |    237 |
| parmap   |  69 |    72 | 112 |          128 |        32787 |  3,521 |
| mandelbrot |  36 |   —  |  —  |       **14** |            — |  1,457 |

Native Grasm matches C on `fib`, which is the benchmark that is almost entirely
function calls and integer arithmetic — the Cranelift call path and the
scalar codegen are doing their job.

Scalar lists (`[i64]`, `[f64]`, `[bool]`) store their elements **inline** — one
machine word each, no per-element heap cell — so `c[i] = c[i] + x` is a load
and a store through the runtime, not an allocation. That change halved sieve
(89→58) and halved matmul twice (187→92), and it is why **sort beats C**
(49 vs 58): Rust's pdqsort over a `Vec<i64>` against C's `qsort` calling a
comparison function through a pointer.

Element access inside a loop is a raw load: a list the loop body *only* indexes
is pinned once (made unique, converted to inline storage if needed) and
accessed through its data pointer. `matmul` fell 99 → 61 → 32ms across two
rounds of that work, and `matmul`'s innermost loop is now 17 instructions with
no calls on the hot path.

Both rounds are worth reporting together, because the first one's stated
conclusion was wrong. This file used to say the remaining gap was the
per-access bounds check "which C at `-O2` does not perform at all". Two
measurements say otherwise:

- Compiling the bounds check out entirely changed `matmul` by **0ms**. The
  branch is perfectly predicted, so it is free.
- Compiling the C with `-fno-vectorize -fno-slp-vectorize` also changed it by
  **0ms**, so the absence of SIMD was not the gap either.

The actual cost was refcount traffic: the pinned load was still preceded by
evaluating the list identifier — an increment — and followed by the matching
decrement, for a value the load never used. Four indirect calls per iteration,
eight million iterations. Removing them took `matmul` from 61ms to 32ms. The
lesson is filed under "read the disassembly before naming the bottleneck".

What is left:

- `wordfreq` is 2.2× off C and the widest remaining gap. A sample profile puts
  about 60% of self time in `malloc` and `free`, not in hashing — so the work
  done since has been removing allocations rather than tuning the hash. Three
  went: the copy of the word "Some" that every `Option` carried, the key copy
  every lookup made to look at a key it did not keep, and the key copy on
  overwriting an existing entry. 149ms to 117ms. What is left is inherent to
  the program — it builds two strings per iteration — and to boxing every
  value on the heap. CPython interns short strings and caches their hashes,
  which is why it is unusually competitive here.
- `sieve` is 1.4× off C on a loop that is memory-bound at 3.4MB.

## The "faster than Python" question

Claims like "100x faster than Python" come from one workload class: tight
numeric kernels with no allocation, where CPython is at its worst. Mandelbrot
is the canonical example, so it is measured here rather than argued about.

| implementation | time | vs Python |
|----------------|-----:|----------:|
| grasm `par_map` |  14 ms | **102x** |
| C, `-O2`, one core | 36 ms | 40x |
| grasm, one core | 39 ms | 37x |
| Python | 1,457 ms | 1x |

Two honest readings of that table. Single-core, Grasm is **1.1x off C** on
float arithmetic — the codegen is sound. And the headline 102x is only
reachable because `par_map` spreads it across cores; C's 40x is one core.

The number to distrust is any single multiplier. Across the whole suite
Grasm is **1.3x to 30x** faster than Python depending entirely on the
workload — 30x on `matmul`, and only 1.3x on `wordfreq`, which is dominated by
allocating strings per iteration rather than by anything the compiler
controls. A language that quotes one number from its best benchmark is
quoting marketing.

## Multi-core

`parmap` is the one row where the languages are asked for *throughput*, each
using its natural idiom: pthreads in C, `std::thread::scope` in Rust,
goroutines in Go, `multiprocessing.Pool` in Python (the GIL rules threads out),
and `par_map` in Grasm.

Grasm gets a **5.9× speedup on 8 cores** — measured A/B against the identical
program with `map` in place of `par_map`: 644 ms to 108 ms, byte-identical
output. That puts it at 133 ms against Go's 116 ms and 27× ahead of Python.

The interesting part is not the number, it is what makes it *safe*:

- The function must be **effect-free, and the compiler proves it**. Put a
  `print` inside and the build fails with `par_map needs an effect-free
  function`. No `Send`/`Sync` bounds to satisfy, no race detector to run —
  the effect system already knew.
- Results come back in **input order regardless of core count**
  ([E-PAR-MAP-ORDER]), so the answer never depends on the machine. Verified
  by running the same binary 40 times and checking exactly one distinct
  result comes back.

`parallel { spawn { … } }` deliberately stays single-core. Its tasks share the
enclosing scope by design ([E-SPAWN-SHARED]), so running them concurrently
would race — structure and determinism there, throughput here.

Two things the first run of these benchmarks found and fixed:

- **The native map was O(n) per operation.** Entries are kept in sorted key
  order so iteration is deterministic, but lookup scanned linearly, making
  whole programs quadratic. `wordfreq` was *slower natively than in the
  interpreter* — 1430ms against 460ms — which is the sort of result that means
  a bug rather than a trade-off. Binary search over the already-sorted vector
  took it to 183ms.

- **Every boxed `bool` and small `i64` was a fresh allocation.** The sieve
  allocated once per flag write. Interning `true`/`false` and the integers in
  `-128..=1024` as immortal shared cells took `sieve` from 538ms to 84ms.

## Beyond speed

Runtime is one axis. These are the others that can be measured honestly.

### Compile time and binary size

Mean over the six programs. Go is measured with a warm build cache, which is
how it is actually used; cold it is 610 ms/program.

| language | compile ms/program | binary KB |
|----------|-------------------:|----------:|
| Go       |                 52 |     2,340 |
| grasm    |                 66 |       457 |
| C        |                 95 |        33 |
| Rust     |                257 |       444 |
| Python   |                  — |         — |

Grasm compiles about as fast as Go, faster than C, and roughly 4× faster than
Rust, which is what a Cranelift backend with no optimizer passes beyond
`opt_level=speed` should look like. Binaries are Rust's size and a fifth of
Go's, after link-time dead-stripping.

Binary size used to be the weak column at 1,503 KB: every executable carried
the whole statically linked runtime whether it used maps, effects, or
concurrency or not. The linker now dead-strips it (`-dead_strip` on macOS,
`--gc-sections` on Linux), which puts Grasm at Rust's size. What remains is
Rust `std` infrastructure the runtime is built on — the panic hook alone pulls
the whole backtrace stack — and going further than ~440 KB means a `no_std`
runtime or a `build-std` toolchain, neither of which stable Rust offers today.
C at 33 KB stays the honest floor.

### Concision

Lines of code with blanks and comments removed, and non-whitespace characters,
for the same six programs:

| measure | grasm |     C |  Rust |    Go | Python |
|---------|------:|------:|------:|------:|-------:|
| SLOC    |   105 |   116 |    92 |   142 |     55 |
| chars   | 1,250 | 2,639 | 1,726 | 1,785 |  1,025 |

Grasm is the second-densest by characters and third by lines. Python wins
both, which is what a dynamically typed language with comprehensions should
do. Rust wins on lines because iterator chains collapse loops that Grasm
writes out — `matmul` is the clearest case.

An earlier version of this table was worse for Grasm because the programs here
used `while` counters where `for i in 0..n` reads better. That was a fairness
bug in the benchmark suite, not a property of the language, and it is fixed.
The inner loops with a non-unit stride are still `while`, as they would be in
every language listed.

### Learning curve

**Not measured, and not measurable this way.** Learning curve is how long it
takes a person to become productive, which needs people. Nothing here
substitutes for that.

What *can* be counted is the surface someone has to learn:

| language    | reserved words |
|-------------|---------------:|
| Go          |             25 |
| **Grasm**   |         **30** |
| Python 3.14 |             35 |
| Rust 2021   |             39 |
| C23         |             44 |

Grasm's 30 is 27 reserved words plus `parallel`, `spawn`, and `bench`, which
are keywords only in one syntactic position — code already using those names
keeps compiling. That number is the design target from MASTERPLAN §19 (a
kernel of at most 30 constructs), so it is a constraint being met rather than
an outcome being reported.

Treat it as a proxy for *surface*, not for difficulty. A small keyword set
says nothing about whether effect rows or `inout` are easy to learn, and the
honest answer there is that nobody outside this repository has tried.

### Whole-project compile time

The six programs above are one file each, which says nothing about a real
codebase. A generated project — **60 modules, 2,400 functions, ~27,000
lines** — built from scratch in each language, all four producing the same
answer:

| language | from-scratch build |
|----------|-------------------:|
| **grasm**| **201 ms**         |
| Rust     |              846 ms |
| C (`-O2`)|            1,751 ms |
| Go       |            2,160 ms |

Grasm's lead *widens* with size: roughly level with Go on single files, about
10× faster on 27k lines. Typechecking alone is 20 ms. Caveat: the generated
code is uniform and simple, so it exercises throughput rather than the
features (generics, traits, macros) that dominate real Rust build times. No
language here had a warm cache.

### Memory under load

Peak resident set in MB, same six programs:

| program  | grasm |   C | Rust |  Go |
|----------|------:|----:|-----:|----:|
| fib      |   1.3 | 1.2 |  1.4 | 3.7 |
| sieve    |   3.4 | 3.2 |  3.3 | 5.5 |
| matmul   |   2.8 | 2.2 |  2.3 | 4.6 |
| wordfreq |   1.9 | 1.3 |  1.6 | 7.1 |
| sort     |   9.6 | 4.3 |  7.5 | 6.8 |
| parmap   |   4.6 | 4.4 |  4.6 | 7.3 |

Measuring this found a real defect: `sieve` was using **18.7 MB against C's
3.2**, because a `[bool]` stored one machine word per element. Bools now get a
byte each — a 2M-entry sieve is 2 MB instead of 16 MB — which is why inline
storage is not simply a `Vec<i64>`. `sort` remains the outlier, in the same
territory as Rust rather than C's fully in-place `qsort`.

### Not benchmarked

- Learning curve, as above.
- **Incremental** rebuilds. The figures above are all from scratch; Grasm has
  no build cache at all, which flatters it here and would not at scale.
- Error-message quality. 32% of diagnostic sites attach a `help:` line; that
  is a coverage number, not a quality measurement.
- Anything off macOS/arm64 for timing. Correctness *is* checked on a second
  architecture (`scripts/crosstest.sh`, 130 programs on x86-64), and in CI on
  Linux and Windows.

The interpreter is 40-240× slower than the native backend, which is expected
and fine: it exists to define semantics, and `scripts/difftest.sh` checks that
the fast one agrees with it on every program in the repository — including
these five.
