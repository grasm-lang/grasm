# Benchmarks

Five programs, each written the way somebody would naturally write it in that
language — no hand-vectorized C competing against idiomatic Python, and no
Grasm program tuned in ways the others were not.

Run them with `./scripts/bench.sh`. Every implementation prints the same
checksum, and the harness refuses to report a timing when the checksums
disagree: an optimizer that deletes the work being measured is the classic way
a benchmark lies, and a wrong answer is not a fast answer.

## What each program measures

| Program    | Stresses                                       |
|------------|------------------------------------------------|
| `fib`      | function call overhead and integer arithmetic   |
| `sieve`    | list indexing and tight loops                   |
| `matmul`   | nested indexing over a flat list                |
| `wordfreq` | hashing, string handling, map updates           |
| `sort`     | comparison-heavy work over a large list         |

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

Apple M-series, macOS 25.1, best of 3 runs, wall clock in milliseconds. Lower
is better. Reproduce with `./scripts/bench.sh`.

| program  |   C |  Rust |  Go | grasm-native | grasm-interp | Python |
|----------|----:|------:|----:|-------------:|-------------:|-------:|
| fib      |  26 |    26 |  27 |       **28** |         1205 |    152 |
| sieve    |  27 |    29 |  28 |           58 |         2268 |    399 |
| matmul   |  25 |    28 |  31 |           92 |         5821 |    973 |
| wordfreq |  52 |    48 |  39 |          181 |          475 |    152 |
| sort     |  58 |    32 |  70 |       **49** |          324 |    243 |

Native Grasm matches C on `fib`, which is the benchmark that is almost entirely
function calls and integer arithmetic — the Cranelift call path and the
scalar codegen are doing their job.

Scalar lists (`[i64]`, `[f64]`, `[bool]`) store their elements **inline** — one
machine word each, no per-element heap cell — so `c[i] = c[i] + x` is a load
and a store through the runtime, not an allocation. That change halved sieve
(89→58) and halved matmul twice (187→92), and it is why **sort beats C**
(49 vs 58): Rust's pdqsort over a `Vec<i64>` against C's `qsort` calling a
comparison function through a pointer.

What remains of the gap is two things, stated so nobody has to guess:

- Every element access is still a **call into the runtime** (with a
  copy-on-write check), where C compiles to a raw load. Closing that means
  teaching codegen the list's memory layout so hot loops inline the access.
- **Map values are still boxed**, which is most of wordfreq's 3.5×.

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

Mean over the five programs:

| language | compile ms/program | binary KB |
|----------|-------------------:|----------:|
| Go       |                 51 |     2,339 |
| grasm    |                 53 |     1,503 |
| C        |                 63 |        33 |
| Rust     |                151 |       440 |
| Python   |                  — |         — |

Grasm compiles about as fast as Go and roughly 3× faster than Rust, which is
what a Cranelift backend with no optimizer passes beyond `opt_level=speed`
should look like.

Binary size is the weak column. Every executable statically links the whole
runtime — `libgrasm_rt.a` — whether the program touches maps, effects, or
concurrency or not, so a program that prints one integer still carries all of
it. Stripping takes 1,503 KB to about 1,080 KB. C at 33 KB is the honest
comparison, and closing that gap means dead-code elimination across the
runtime, which does not exist yet.

### Concision

Lines of code with blanks and comments removed, and non-whitespace characters,
for the same five programs:

| measure | grasm |   C | Rust |  Go | Python |
|---------|------:|----:|-----:|----:|-------:|
| SLOC    |    84 |  95 |   66 |  95 |     44 |
| chars   | 1,020 | 2,200 | 1,193 | 1,255 |    801 |

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

### Not benchmarked

- Learning curve, as above.
- Incremental and whole-project compile times. The five programs are one file
  each; nothing here says how a large codebase behaves.
- Memory use under load, beyond spot checks.
- Error-message quality. 32% of diagnostic sites attach a `help:` line; that
  is a coverage number, not a quality measurement.

The interpreter is 40-240× slower than the native backend, which is expected
and fine: it exists to define semantics, and `scripts/difftest.sh` checks that
the fast one agrees with it on every program in the repository — including
these five.
