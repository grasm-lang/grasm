# 27 — Full Cryptographic Engine (SHA-256)

**Tests:** bit manipulation, 32-bit arithmetic in a 64-bit language, known-answer
correctness. **Result:** works, all three engines agree. **12 / 12 vectors.**

## Why this project exists

It was **blocked** until [L-LIB-BITS] existed — it is the reason that clause was
added, and it is the strongest possible test of it. SHA-256 is nothing but
rotations, shifts, xors and truncated addition. Every published vector either
matches exactly or does not; there is no partial credit and no plausible-looking
wrong answer.

## What it exercises

* `ushr` and not `shr` in `rotr32`. An arithmetic shift smears the sign bit
  across the top and every digest comes out wrong — this is the single easiest
  way to get SHA-256 subtly broken, and the two methods exist to make it a
  choice rather than an accident.
* Masking to 32 bits by hand, everywhere, because Grasm has no unsigned 32-bit
  type. Verbose, and honest: the masking is where a reimplementation actually
  goes wrong, and writing it out makes it checkable.
* **UTF-8 encoding by hand.** `str` is text and `codes()` gives code points, so
  `"é"` is one code point and must be hashed as its *two* UTF-8 bytes. Hashing
  the code point gives a digest that disagrees with every other SHA-256 in the
  world. The `é` vector is in the suite precisely to catch that, and it passes.
* Five length boundaries — 55, 56, 63, 64, 65 bytes — which are the four places
  a padding bug lives.

## An honest failure

Two expected digests were typed from memory and were wrong; the implementation
disagreed with them and the implementation was right. Every constant in the file
now comes from an independent implementation (Python's `hashlib`), never from
this program's own output — a known-answer test that grades itself is not a test.

## Findings

None. This project consumed F3 rather than producing a finding: the gap it named
was closed to make it writable.
