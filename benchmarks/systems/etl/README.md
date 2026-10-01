# etl — streaming JSON-to-database ETL

One input file of JSON Lines, three implementations (Grasm, Rust, Go) that do
the same thing to it and must print the same line.

## What it measures

Each program:

1. **Streams** the file a line at a time. Nothing loads the whole file.
2. **Parses** each line as JSON into a strongly typed record:
   `Customer{id, user{name, email, tags[], address{city, zip}}, orders[{sku, qty, price}], note}`.
3. **Validates**: every field present with the right JSON type, `email`
   contains `@`, every order has `qty > 0` and `price >= 0`. A line that fails
   any of these is *rejected* and counted.
4. **Transforms** a valid record into a row
   `{id, name, city, order_count, total_cents = Σ qty·price, tag_count}`.
5. **Batches** rows by 1000 and **inserts** each batch into a mock database:
   per-city aggregate (count, Σ total_cents), a grand total, and a running
   checksum over the rows in order. The batch is dropped after the insert.

Output (one line, identical across languages):

```
valid=N rejected=N cities=N total_cents=N checksum=N city_digest=N
```

`checksum` is `(c · 1000003 + id·7919 + total + order_count·31 + tag_count·17 + bytelen(name)) mod 1e9+7`
folded over every row in file order; `city_digest` folds `(d·131 + count·7 +
total + bytelen(city)) mod 1e9+7` over the cities in byte order. Together they
catch a wrong field, a wrong row, a mis-decoded name or city, and a wrong
aggregate.

The point of the test is the combination of parsing throughput and **bounded
memory**: peak RSS should not grow with the file.

## Input

`gen/` (Go) writes the file deterministically from a seed (splitmix64; same
seed → byte-identical file). Each line is ~610 bytes: a nested user, 0–5
orders, a 150–500 character note containing UTF-8 text and JSON escapes
(`\"`, `\n`, `\t`, `\\`, `\/`, `é`, `中`), non-ASCII names and
cities, and 1 line in 20 carries an unknown extra field every decoder must
ignore. Prices are integer cents, so no float is involved anywhere.

3.0% of lines are invalid by construction, spread over six kinds: a required
field missing (`id`, `email`, `address`, `city`, `orders`, `qty`, or `note`),
an email without `@`, a `qty` of 0 or negative, `id` as a string, `price` as
a string, and `tags` as a string instead of a list. The generator prints how
many it wrote; `rejected` must equal it (2468 of 81 562 lines at 50 MB).

## Sizes

| | size | lines |
|---|---|---|
| verify small | 2 MB | 3 264 |
| verify medium | 50 MB | 81 562 |
| `run.sh` default | 1 GiB | ~1.75 M |
| original spec | 10 GB | ~16 M |

The original spec asks for 10 GB. The machine this was built on had ~14 GB
free, so the default is **1 GiB** and the size is a parameter:
`./run.sh 10000000000` produces and runs the original. Every per-line cost is
independent of position in the file and the aggregate is bounded (480
cities), so wall time scales linearly with size and peak RSS should not move
at all; a 1 GiB run says what a 10 GB run would, divided by ~9.3. `run.sh`
deletes any other `etl-full-*.jsonl` before generating, so at most one large
input exists at a time.

## Running

```
./verify.sh            # builds all, runs 2 MB and 50 MB, fails unless outputs match
./run.sh [SIZE_BYTES]  # builds all, generates input if absent, one timed run each
```

`run.sh` prints `name wall_ms rss_mb output` per language (RSS from
`/usr/bin/time -l`, whose "maximum resident set size" is bytes on macOS) and
exits non-zero if any output differs. Each binary is run once on a tiny input
first so macOS's one-off first-exec cost is not charged to it. Paths:
`ETL_DATA` (input dir), `OUT` (binaries), `CARGO_TARGET_DIR`, `GOCACHE`,
`SEED` (default 42).

## The three implementations

**Rust** (`rust/`): `BufReader::read_line` into a reused `String`,
`serde_json::from_str` into `#[derive(Deserialize)]` structs. serde rejects a
missing field and a wrong type by itself; the two value checks are explicit.
`BTreeMap` for the city aggregate.

**Go** (`go/`): `bufio.Scanner`, `json.Unmarshal` into tagged structs.
`encoding/json` leaves a missing field at its zero value rather than failing,
so required fields are pointers (and slices checked for nil) and `decode`
checks them — the standard Go way to tell "absent" from "zero". A wrong type is
an `UnmarshalTypeError`. The batch slice is reused (`batch[:0]`) after each
insert, which is the Go idiom for dropping its contents.

**Grasm** (`grasm/`, a package depending on `packages/json`): the language has
no line reader for files, so the program reads 1 MiB with `read_bytes_at`,
cuts at the last newline byte (so a UTF-8 character is never split across
chunks), converts that slice to text and walks `lines()`; the next read starts
after the newline. A line longer than the chunk doubles the chunk. Each line
is decoded straight into the typed structs with the `json` package's cursor
(`open_object`, `next_key`, `read_str`, `read_int`, `skip`, `finish`) --
hand-written, since `json` has no derive -- so no tree is built, as serde and
`encoding/json` build none. Members may come in any order, unknown ones are
skipped but still checked, a repeated key keeps its first value, and a missing
field or wrong type is a `JsonError` that rejects the line.

## Deviations, and why

- **Grasm used to build a tree per line** with `json.parse`, walk it, and
  release it by hand (`json.free`); the tree was most of the run (1.6 s at
  50 MB). Reading with the cursor allocates only what the record keeps, so
  there is nothing to release, and a malformed line leaks nothing either.
- **Go's required-field check** is explicit code, for the reason above. It is
  part of what idiomatic Go has to do, so it is counted.
- **Numbers**: `json.read_int` reads the digits as an `i64`, exactly, and
  refuses a fraction or an exponent (`2.0`, `1e1`) as serde and
  `encoding/json` do. The tree decoder went through `f64` and so accepted
  `2.0` as 2 and refused any integer past 2^53; neither occurs in the
  generated input.
- **Grasm reads the file with one `open` per 1 MiB chunk** (`read_bytes_at`
  takes a path, not a handle): ~1000 opens per GiB.
