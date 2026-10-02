# Systems suite — results, 2026-10-02

One run of each `run.sh` at full size, back to back, on an 8-core Apple
machine (6 performance + 2 efficiency cores) at load 3–5. One run is not a
distribution: treat differences under ~10% as noise. Compiler at e7524a4.

Grasm does **not** beat Rust across this suite. It wins `chat`, is within 10%
on `ecs` and `proxy` throughput, and loses `etl`, `dirtyheap` and the proxy's
tail latency.

| benchmark | metric | Grasm | Rust | Go | Grasm / Rust |
|---|---|---|---|---|---|
| chat | deliveries/s | **614,162** (exact) | 194,689 (not exact) | 196,645 (not exact) | 3.15x better |
| chat | peak RSS MB | **7.8** | 60.6 | 292.9 | 7.8x better |
| ecs (1M × 100) | wall ms | 234 (aos), 230 (soa) | **214** | 265 | 1.09x worse |
| ecs | RSS MB | 47.3 | 47.2 | 50.1 | parity |
| etl (1 GiB) | wall ms | 9,231 | **2,560** | 9,536 | 3.6x worse |
| etl | RSS MB | 5.5 | **4.3** | 10.2 | 1.3x worse |
| dirtyheap (5M) | wall ms | 2,770 (values) | **1,070** (arena), 2,290 (Rc) | 1,590 | 2.6x worse (1.2x vs Rc) |
| dirtyheap | drop ms | 497 | **206** (arena) | 25 (GC, concurrent) | 2.4x worse |
| dirtyheap | RSS MB | 1,081 | **789** | 942 | 1.4x worse |
| proxy (256 conns) | req/s | 56,242 | **61,783** | 52,616 | 0.91x |
| proxy | p99 ms | 10.97 | **8.36** | 15.82 | 1.31x worse |
| proxy | p999 ms | 17.87 | **12.34** | 102.18 | 1.45x worse |

What is behind each loss, measured rather than guessed:

- **etl**: Grasm makes a value per string field and validates its UTF-8 per
  slice; Rust's serde decoder borrows from the line. Almost half of Grasm's time
  is its own generated code, a fifth the runtime, the rest allocation and
  copying.
- **dirtyheap**: each node's URL and edge list is a box of its own, so the
  BFS takes two cache misses per node that Rust's arena (URL length and edge
  pointer inline in the node) does not. Rust written with Grasm's
  representation is within 1.2x.
- **proxy tail**: a connection stays on the worker that accepted it, so a
  paused worker stalls its clients; tokio moves runnable tasks to idle
  threads. See `proxy/README.md`.
