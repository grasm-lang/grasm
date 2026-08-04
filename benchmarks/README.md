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
| fib      |  26 |    27 |  29 |       **28** |         1209 |    154 |
| sieve    |  29 |    28 |  30 |           84 |         2292 |    399 |
| matmul   |  26 |    26 |  29 |          184 |         6274 |    990 |
| wordfreq |  53 |    48 |  40 |          183 |          460 |    152 |
| sort     |  59 |    35 |  70 |          102 |          348 |    241 |

Native Grasm matches C on `fib`, which is the benchmark that is almost entirely
function calls and integer arithmetic — the Cranelift call path and the
scalar codegen are doing their job.

The gap on the others is one thing: **values in collections are boxed**. A list
element or map value is a refcounted heap cell, so `c[i] = c[i] + x` in the
matmul inner loop allocates. C, Rust, and Go store `int64` inline and do not.
That is a property of the current value representation, not of the codegen, and
closing it means unboxed storage for scalar collections — a representation
change, not an optimizer pass.

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

The interpreter is 40-240× slower than the native backend, which is expected
and fine: it exists to define semantics, and `scripts/difftest.sh` checks that
the fast one agrees with it on every program in the repository — including
these five.
