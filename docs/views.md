# Borrowed string views: the ceiling, measured before building

`etl` is Grasm's worst benchmark -- 10,260 ms against Rust's 2,854. The
standing explanation is that Grasm makes an owned, UTF-8-validated string value
per decoded field where Rust's serde borrows from the line. The obvious fix is
a borrowed view type. Before writing one, the usual question: **what is it
worth at its ceiling?**

## The model

`scratchpad/viewmodel` decodes the same JSON Lines bytes twice, with the same
cursor, the same fields read, the same per-city aggregate and the same
checksum. The two decoders differ in one thing:

- **owned** -- every string field becomes an allocated, validated `String`,
  and the city map is keyed by an owned `String`. This is `json.read_str`.
- **view** -- every string field is a slice of the line, validated once per
  line, and the city map is keyed by the slice, allocating only on a new city.
  This is the *most* a view type could do; a real one reaches some fraction.

Both decoders are asserted to agree on rows, total, checksum and city count
every iteration, so the comparison is between two correct decoders. Runs are
interleaved and the median of five taken, because sequential runs drift.

## What it said, 2026-10-03

| lines | owned | view | ratio |
|---|---|---|---|
| 200,000 | 140.5 ms | 84.1 ms | 0.598x |
| 400,000 | 276.6 ms | 170.0 ms | 0.615x |
| 600,000 | — | — | 0.610x |

**A view saves about 39% of the decode loop**, steady across sizes. That is a
real optimisation and the largest single one left in `etl`.

It is not, however, what a view was claimed to be worth. Tripling `etl`'s
speed needs 67% of its time removed; string materialisation is a third of it.
Applied to the whole program, 10,260 ms becomes roughly 6,500 -- still about
2.3x Rust, because the other half of the profile is Grasm's own generated code,
which a view type does not touch. Closing `etl` needs the view *and* the
backend; the view alone moves it from 3.6x to about 2.3x.

## What this does not say

The 39% is the ceiling of the representation change in a language that already
has borrowing. Grasm would also need the lifetime rule that makes it safe --
a view may not outlive the buffer it points into -- and whatever that rule
costs in refused programs is not measured here. `lent` parameters, modelled
the same way on `tree` in September, came out 5.7% *worse* than what Grasm does
now, so a borrowing mechanism is not free by assumption.
