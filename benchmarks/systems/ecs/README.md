# ecs — entity-component-system update loop

What a game engine's movement system does every frame: walk every entity,
integrate its position by its velocity, bounce it off the world's walls. It
measures a tight loop over a large array of small records of `f64` — memory
layout, field access, and in-place update — with almost no arithmetic per
element.

## The workload

- `N` entities (default 1,000,000), each with `x, y, z, vx, vy, vz: f64`.
- Initial values come from the integer LCG `s = (s * 1103515245 + 12345) % 2^31`
  (seed 42): positions `(s % 2000001 - 1000000) / 1000.0`, velocities
  `(s % 2001 - 1000) / 10.0`, drawn in the order x, y, z, vx, vy, vz per
  entity. Floats are derived from integers, so every language starts from the
  same bits.
- `S` steps (default 100), `dt = 0.01`: `p = p + v * dt` for each axis; if
  `p > 1000` then `p = 2000 - p` and `v = -v`, if `p < -1000` then
  `p = -2000 - p` and `v = -v`.
- Output, one line: `checksum <sum of x+y+z, 6 decimals> <sum of floor(c*1000) over every coordinate, as i64>`.
  The integer half is deliberately sensitive to the last bit of every
  coordinate, so it catches any variant that rounds differently.

## Variants

| name | file | layout |
|------|------|--------|
| `grasm_aos` | `grasm/ecs.gr` | `type Entity = {…}`, `[Entity]`; each step reads `var e = es[i]`, updates its fields, writes `es[i] = e` |
| `grasm_soa` | `grasm/ecs_soa.gr` | six `[f64]` lists (struct-of-arrays), how a Grasm programmer optimises this today |
| `rust` | `rust/` | `Vec<Entity>`, `#[derive(Clone, Copy)]`, `iter_mut()` |
| `go` | `go/` | `[]Entity`, updated through `&es[i]` |

All take `[N] [S]` on the command line.

## Running

```
./verify.sh          # builds everything; checks outputs agree at N=1000 S=5,
                     # N=50000 S=10, N=3 S=1000 (+ grasm run at the small sizes)
./run.sh             # builds, runs each variant once at N=1000000 S=100
./run.sh 50000 10    # any other size
```

Both source `build.sh`, which builds all four into `BUILD_DIR`.
`run.sh` prints name, wall-clock ms, peak RSS (MB, from `/usr/bin/time -l`)
and the checksum, and exits 1 if any checksum differs. Before timing it runs
every binary once at `N=1 S=1`: on macOS a freshly written executable pays a
one-off ~0.5 s launch cost on its first run, and the scripts rebuild every
time.

Environment: `GRASM` (default `../../../target/release/grasm`), `BUILD_DIR`
(default `$TMPDIR/grasm-systems-ecs`), `CARGO_TARGET_DIR` (default under
`BUILD_DIR`); `GOCACHE` etc. are honoured as usual. Rust builds `--offline`
with no dependencies; Go uses only the standard library.

## Deviations and notes

- **Go needs `float64(…)` around `v * dt`.** The Go spec allows the compiler to
  fuse `x + y*z` into one FMA, and on arm64 it does; that rounds once instead
  of twice and changes the integer checksum (`-63015656` instead of
  `-63015691` at N=50000 S=10). An explicit conversion is the spec's
  documented way to forbid the fusion. Rust and Grasm (Cranelift) do not fuse.
- The LCG is a 31-bit one so that `s * 1103515245` cannot overflow `i64` in any
  language, which keeps the arithmetic identical without bit operations.
