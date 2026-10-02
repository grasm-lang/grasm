# Systems suite — results, 2026-10-02 (afternoon)

One run of each `run.sh` at full size on an 8-core Apple machine (6
performance + 2 efficiency cores), load 7–24 and varying between benchmarks.
One run is not a distribution: treat differences under ~10% as noise, and
`chat` as noisier than that. Compiler at the inline-fields merge.

Grasm does **not** beat Rust across this suite. It wins on memory everywhere
-- by 8x on `chat` and 5x on `proxy` -- and its `dirtyheap` drop is faster
than Rust's arena. It loses `etl`, `dirtyheap`, `ecs` and `chat` on time, and
the `proxy` tail to both Rust and Go.

| benchmark | metric | Grasm | Rust | Go |
|---|---|---|---|---|
| chat | deliveries/s (median of 3) | 590,078 | **693,534** | 131,735 |
| chat | peak RSS MB | **8.4** | 69.4 | 264.0 |
| ecs (1M × 100) | wall ms | 277 | **238** | 313 |
| ecs | RSS MB | 47.3 | **47.2** | 50.1 |
| dirtyheap (5M) | wall ms | 1,860 | **1,240** | 2,720 |
| dirtyheap | drop ms | **266** | 326 | 83 (GC, concurrent) |
| dirtyheap | RSS MB | **718** | 789 | 914 |
| etl (1 GiB) | wall ms | 10,260 | **2,854** | 10,429 |
| etl | RSS MB | 5.7 | **4.0** | 10.6 |
| proxy (256 conns) | req/s (median of 3) | 42,045 | 40,159 | **47,563** |
| proxy | p99 ms (median of 3) | 29.1 | 27.0 | **18.5** |
| proxy | RSS MB | **7.2** | 8.3 | 37.1 |

Changes since the morning run: `dirtyheap` 3067 → 1860 ms and 1128 → 718 MB,
from holding a value struct's string and scalar-list fields inline instead of
in a box of its own (docs/values.md). Its drop is now faster than Rust's
arena, and it uses less memory than Rust.

`chat` and `proxy` were re-run three times each on a quieter machine, and
both tables above now show the median; the other rows are single runs. That
changed two conclusions a single run had given:

- **chat**: Rust is ahead by about 1.18x, consistently (590k against 694k),
  not level. The morning run where Grasm appeared to win was one where Rust
  failed to deliver 4,807 of its messages. Grasm delivered all of them every
  time, in 8.4 MB against Rust's 69 MB.
- **proxy**: Go leads the tail (p99 18.5 ms against Grasm's 29.1 and Rust's
  27.0), where a single run had put Grasm ahead of Go. Throughput swings from
  29k to 48k for the same binary, because the load generator, the upstream and
  the proxy all compete for the same eight cores: this row cannot separate the
  three at better than about 20%.

What is behind each loss, measured rather than guessed:

- **etl**: Grasm makes a value per string field and validates its UTF-8 per
  slice; Rust's serde decoder borrows from the line. Almost half of Grasm's
  time is its own generated code, a fifth the runtime, the rest allocation and
  copying. Borrowed string and bytes views are the next lever.
- **dirtyheap**: what is left is the build, where the URL is made as a string
  value and then copied into the field; Rust formats it into place.
- **ecs**: the update loop is at parity; the setup loop is behind on
  instructions retired.
- **proxy tail**: a connection stays on the worker that accepted it, so a
  paused worker stalls its clients; tokio moves runnable tasks to idle
  threads. See `proxy/README.md`.
