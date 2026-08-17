# 18 — Image Format Converter

**Tests:** binary stream reading, byte arrays, bitwise manipulation.
**Result:** works, all three engines agree. **Was blocked; is the reason `bytes`
exists.**

## What unblocked it

F9 — no byte type. The demonstration that made the case was seven bytes of x86
`mov rax, 1` becoming **nine** bytes on disk, because `str` is text and re-encodes
anything above 127 as UTF-8. A BMP header is not text.

[L-LIB-BYTES] fixed that, and this project is the proof it was the right fix.

## What it does

Writes a 24-bit BMP, reads it back, parses the header field by field, converts to
greyscale, writes a second BMP, reads *that* back, and checks the arithmetic.

**The image is a real BMP, and an outside tool says so:**

```
$ file out.bmp
out.bmp: PC bitmap, Windows 3.x format, 3 x 2 x 24, image size 24,
         resolution 2835 x 2835 px/m, cbSize 78, bits offset 54
```

An independent Python decoder reads the top row back as
`[(255,0,0), (0,255,0), (0,0,255)]` — exactly what was written.

## The parts that are actually hard

* **Row stride.** BMP pads every row to a multiple of four bytes. The image is
  3 pixels wide — 9 bytes per row, needing 3 of padding — chosen because that is
  the case a stride bug gets wrong. At width 4 the bug is invisible.
* **Bottom-up rows and BGR order.** BMP stores the last row first and each pixel
  as blue, green, red. Both are inverted on read and on write.
* **Little-endian fields** built and parsed with `shl`/`ushr`/`bit_and`
  ([L-LIB-BITS]), which is the other clause this project needed.
* **Integer greyscale.** Rec. 601 luma with weights scaled by 1000 and divided
  back, so the answer does not depend on float rounding and is identical on
  every engine. Checked against arithmetic done by hand — `255,0,0` → 76,
  `128,64,32` → 79 — not against the program's own output.

Five malformed inputs are refused with a reason rather than crashing: empty, too
short, wrong magic, 8-bit colour, and a valid header whose pixel data was cut off.

## Findings

None. This project consumed F9 rather than producing a finding.
