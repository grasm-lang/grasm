# Borrowed string views: the ceiling, and what it was hiding

`etl` is Grasm's worst benchmark -- 10,260 ms against Rust's 2,854. The
standing explanation is that Grasm makes an owned string value per decoded
field where Rust's serde borrows from the line, and the fix is a borrowed view
type. Before writing one: what is it worth at its ceiling?

## The model, twice, because the first one was wrong

`scratchpad/viewmodel` decodes the same JSON Lines twice with the same cursor,
the same fields, the same per-city aggregate and the same checksum, asserting
the two agree on every count each iteration.

The **first** model made each field a Rust `String` and measured a 39% saving.
That model was unfaithful: `INLINE_STR_CAP` is 38 bytes and **every field in
this data fits** -- city 14, email 25, sku 10, price 6 -- so Grasm's string
value heap-allocates nothing for the text, where `String` always does. Written
faithfully, with a box per field and the text inline, the saving came out
45-48%. Two models, two answers, both of a decoder rather than of `etl`.

## The real program, which is the only oracle that counts

`json.skip` on a string is `scan_string(p, false)`: the same scan, the same
validation, differing from `read_str` only in whether a value is made at the
end. So the cost of *making* the values can be measured by not making them.

Three fields -- `zip`, `sku`, `note` -- are read, stored, and never looked at
again, so skipping them leaves the output byte-identical (verified) and the
city map untouched. Five interleaved rounds on 256 MiB:

| | median | min |
|---|---|---|
| all seven fields made | 2.54 s | 2.48 s |
| three of them not made | 1.96 s | 1.90 s |

**23% of `etl` is spent making three of its seven string fields.** That is the
number to design against, and it is a real-program number.

A first attempt at this bound was wrong in a way worth recording: replacing
*every* field with a constant also collapsed the city map from 480 keys to
one, so the 35% it reported included map work. A bound has to remove one
thing.

## What shipped, 2026-10-03

The bound sent me looking for where 23% could possibly go, since a box is a
free-list pop and a fallible call's `Ok` measured 1.5 ns. It was simpler than
any of that:

```rust
fn text_of_bytes(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(t) => t.to_owned(),   // a heap buffer, for text that fits inline
        ...
```

`grasm_bytes_slice_to_str` -- the fused `b.slice(i, j).to_str()` that every
parser in the language runs once per token -- built a `String`, handed it to
`RtStr::new`, which copied it into the value's own 38 bytes and freed it
again. `RtStr::of` has existed all along for exactly this, and its own doc
comment warns against the spelling that was being used three lines away.

Taking the allocation out is **5% of `etl`** (median 2.50 -> 2.38 s, min
2.46 -> 2.33, output identical) and applies to every `to_str` in the language.

`bytes.slice` was separately made a window on its buffer rather than a copy,
which `RtBytes` was already shaped for -- `ptr`, `len` and an `Arc` owner,
with `as_slice` ignoring the first two. It is worth nothing on `etl`, whose
hot path is fused and never builds the intermediate value, and **197 ms -> 0
ms** on 2,000 slices of a 4 MB buffer, with RSS slightly lower. O(n) to O(1),
visible only where the buffers are big.

## What is left, and what it would take

5% of the 23%. The rest is the value itself -- the box, the inline copy, the
reference counting -- and only a view removes that, which needs the rule that
a view may not outlive its buffer. `lent` parameters, modelled the same way on
`tree` in September, came out 5.7% *worse* than what Grasm does now, so a
borrowing mechanism is not free by assumption.

Even at its ceiling the view leaves `etl` around 2.3x Rust, because the other
half of the profile is generated code a view never touches. Closing `etl`
needs the view *and* the backend.
