# 18 — Image Format Converter — **blocked**

**Tests:** binary stream reading, byte arrays, bitwise manipulation.

**Result:** cannot be written in Grasm today.

## Why

Three separate gaps, each sufficient on its own:

1. **No byte type** (FINDINGS.md F9). `read_file` returns `str`. A BMP is not
   text, and a `str` in Grasm is UTF-8 — so the header bytes of a real BMP would
   either fail to decode or be silently replaced.
2. **No byte length** (F2). `str.len()` counts code points, so even if the bytes
   arrived intact, the width and height fields could not be located by offset.
3. **No bitwise operators** (F3). A BMP's pixel data is packed; extracting a
   5-bit channel from a 16-bit word needs `>>` and `&`, and neither exists.
   Division and modulo can stand in for shifts and masks on non-negative values,
   which is what project 16's hash had to do — but that is a workaround for
   arithmetic, and there is nothing to work around here, because the bytes
   never arrive.

## What would unblock it

A byte type with `read_bytes`/`write_bytes`, and bitwise operators. Both are
ordinary work with no design questions attached — a `bytes` value is a list of
`i64` in the range 0..255 as far as the type system is concerned, and the
operators are five new tokens and five new `BinaryOp` variants.

The image *algorithm* — reading a header, walking a pixel array, writing a new
one — needs nothing else the language lacks. Project 05 already does
two-dimensional indexing, and project 11 already does a file round trip.
