# dirtyheap — the deallocation spike

A crawler's URL map: N nodes (default **5,000,000**), each holding a URL
(`https://site{i % 1000}.example/page/{i}`) and 1–9 out-edges (average 5)
chosen by the integer LCG `s = (s * 1103515245 + 12345) % 2^31`, seed 42 — the
same graph in every language. A BFS from node 0 counts reachable nodes and sums
their URL lengths; that is the checksum. Then the whole graph is dropped at
once, and the drop alone is timed with a monotonic clock.

The program prints `checksum …`, `build_ms`, `drop_ms`, `total_ms` (Go adds
its `gc_*` lines). `run.sh` adds the external wall time and peak RSS from
`/usr/bin/time -l` (macOS reports bytes; shown as MB).

## Variants

| variant | representation | what "drop" is |
|---|---|---|
| `grasm/refs.gr` | Tier-2 `ref Node`, edges are `[ref Node]` | `release()` every node, then `graph = []`. `ref` reclamation is manual ([T-REF-GENERATIONAL]), so releasing every node *is* how a ref graph is torn down. |
| `grasm/values.gr` | `[Node]` of refcounted structs, edges are `[i64]` indices | `graph = []`: the last reference goes and every node, URL and edge list is freed. |
| `rust` `dirtyheap` | arena: `Vec<Node>`, `usize` edges | `drop(graph)` |
| `rust` `dirtyheap_rc` | `Rc<RefCell<Node>>`, strong `Rc` edges | clear every node's edges (the graph is cyclic; without this the Rcs leak), then drop the `Vec`. The Rust analogue of the refs variant. |
| `go` | `[]*Node`, pointer edges | `graph = nil; runtime.GC()` |

### Reading Go's number honestly

Go's `drop_ms` is the wall time of a forced `runtime.GC()` after the root is
cleared. Most of that collection runs concurrently; the stop-the-world part is
reported separately as `gc_drop_stw_pause_ms` (the delta of
`MemStats.PauseTotalNs` across the call), and `gc_total_pause_ms` is every
pause in the run, including the collections during the build. Without the
forced `runtime.GC()` the garbage would be reclaimed at some later, unscheduled
point — so the Go figure answers "how long does a collection of this heap
take", not "how long does freeing take", and Go hands the pages back to the OS
lazily (the scavenger), which no number here shows.

### Clock precision

Grasm's only monotonic clock is `monotonic_millis()` ([L-LIB-CLOCK]); there is
no nanosecond builtin, so Grasm's `drop_ms` is whole milliseconds. At full size
the drop is hundreds of ms, so this does not decide anything, but a sub-ms drop
reads as 0. (A native-only workaround exists — `extern fn
clock_gettime_nsec_np(id: i64) -> i64` inside `unsafe` — and is not used, since
it would put an `io, unsafe` C call in a benchmark that should be idiomatic.)
Rust and Go report the drop to the microsecond.

## Size and memory

N = 5,000,000 is kept. Measured peak RSS at N = 1,000,000 was ~340 MB for the
worst variant (`grasm-refs`) and scales linearly, so the full size peaks around
1.7–1.8 GB, under the 4 GB budget.

## Scripts

- `./verify.sh` — builds everything and checks the checksums agree at N = 1,
  1,000 and 100,000 (`SIZES="…"` to change).
- `./run.sh` — builds everything and runs each variant once at N (default
  5,000,000; `N=… ./run.sh`), printing wall ms, drop ms, program ms, peak RSS
  MB and checksum; exits non-zero if any checksum differs. Each binary is run
  once at N = 1 first so macOS's one-off first-exec check is not in the row.
- `build.sh` — shared by both; binaries go to `$OUT` (default
  `$TMPDIR/dirtyheap-bin`), Rust's target dir to `$CARGO_TARGET_DIR`.

Each language is built the usual way: `grasm build --native`, `cargo build
--release` (std only), `go build` (stdlib only, go 1.25).
