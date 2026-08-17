# 16 — Key-Value In-Memory Database
**Tests:** hash function, bucket arrays, collision chaining, resize. **Result:** works, all three engines agree.

`Map[K, V]` exists, so using it would test nothing. This builds the table:
FNV-1a, separate chaining, load-factor 0.75 resize — with the library map kept
alongside as an **oracle**, so a collision bug shows up as a disagreement rather
than as a lookup that quietly returns the wrong value.

Collisions are not left to chance. `colliding_keys` searches for five keys that
land in the same bucket, because a table with 16 buckets and 8 well-spread keys
never exercises the chain at all. It found `[k15, k59, k60, k73, k82]`, all five
resolve, deleting the middle one leaves the others intact.

FNV-1a is where `[E-ARITH-INT]` earns its keep — the multiply overflows on
nearly every byte and is *supposed* to. With no `^` operator (F3) the xor is
done arithmetically, bit by bit, which is ugly and correct.

After 100 inserts starting from 4 buckets: 256 buckets, load factor under 0.75,
every key survived every resize, longest chain 3. Empty string, space, newline,
`café` and 🙂 all round-trip as keys.

## Findings
None new. F3 (no bitwise operators) forced the arithmetic xor.
