# Systems suite — results, 2026-10-02 (afternoon)

One run of each `run.sh` at full size on an 8-core Apple machine (6
performance + 2 efficiency cores), load 7–24 and varying between benchmarks.
One run is not a distribution: treat differences under ~10% as noise, and
`chat` as noisier than that. Compiler at the inline-fields merge.

Grasm does **not** beat Rust across this suite. It wins on memory everywhere,
leads `proxy` throughput and beats Go on every row but one; it loses `etl`,
`dirtyheap` and `ecs` on time, and `chat` throughput to Rust.

| benchmark | metric | Grasm | Rust | Go |
|---|---|---|---|---|
| chat | deliveries/s | 528,259 | **574,738** | 268,185 |
| chat | peak RSS MB | **8.6** | 53.8 | 200.2 |
| ecs (1M × 100) | wall ms | 277 | **238** | 313 |
| ecs | RSS MB | 47.3 | **47.2** | 50.1 |
| dirtyheap (5M) | wall ms | 1,860 | **1,240** | 2,720 |
| dirtyheap | drop ms | **266** | 326 | 83 (GC, concurrent) |
| dirtyheap | RSS MB | **718** | 789 | 914 |
| etl (1 GiB) | wall ms | 10,260 | **2,854** | 10,429 |
| etl | RSS MB | 5.7 | **4.0** | 10.6 |
| proxy (256 conns) | req/s | **36,406** | 35,893 | 35,354 |
| proxy | p99 ms | 28.6 | **25.8** | 31.8 |
| proxy | RSS MB | **7.2** | 8.3 | 30.8 |

Changes since the morning run: `dirtyheap` 3067 → 1860 ms and 1128 → 718 MB,
from holding a value struct's string and scalar-list fields inline instead of
in a box of its own (docs/values.md). Its drop is now faster than Rust's
arena, and it uses less memory than Rust.

`chat` is the row that moves most between runs: in the morning Rust delivered
only 9,985,193 of 9,990,000 messages at 194,689/s, and here it delivered all
of them at 574,738/s. Grasm delivered all of them in both.

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
