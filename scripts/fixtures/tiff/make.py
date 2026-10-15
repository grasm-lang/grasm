#!/usr/bin/env python3
"""Build the TIFF fixtures `scripts/tifftest.sh` decodes.

    python3 scripts/fixtures/tiff/make.py

Needs Pillow. The output is committed, so this only runs when the cases
change.

Most of them Pillow writes. Two it will not, and those are the two most
likely to be wrong in a decoder nobody tested against them:

* **big-endian** -- Pillow emits `II` only, so `be_rgb.tif` is assembled here
  byte by byte, and `le_rgb.tif` beside it is the same image the other way
  round so a failure says which half is wrong.
* **predictor 2** -- Pillow's `tiff_adobe_deflate` sets no predictor, so
  `pred_rgb.tif` stores horizontal differences and sets tag 317 itself.

Every hand-built file is read back through Pillow before it is written, so a
fixture cannot be wrong in the same direction as the decoder it tests.
"""

import os
import struct
import zlib

from PIL import Image

OUT = os.path.dirname(os.path.abspath(__file__))


def gradient(w, h, mode="RGB"):
    im = Image.new(mode, (w, h))
    px = im.load()
    for y in range(h):
        for x in range(w):
            if mode == "L":
                px[x, y] = (x * 13 + y * 7) % 256
            else:
                px[x, y] = (
                    x * 255 // max(1, w - 1),
                    y * 255 // max(1, h - 1),
                    (x ^ y) % 256,
                )
    return im


PILLOW_CASES = [
    ("raw_rgb", 40, 24, "RGB", "raw"),
    ("lzw_rgb", 40, 24, "RGB", "tiff_lzw"),
    ("deflate_rgb", 40, 24, "RGB", "tiff_deflate"),
    ("packbits_rgb", 40, 24, "RGB", "packbits"),
    ("raw_gray", 32, 16, "L", "raw"),
    ("lzw_gray", 32, 16, "L", "tiff_lzw"),
    # Not a multiple of anything.
    ("raw_big", 17, 9, "RGB", "raw"),
]

W, H = 12, 8
PW, PH = 16, 10


def flat(w, h, f):
    d = bytearray()
    for y in range(h):
        for x in range(w):
            d += bytes(f(x, y))
    return bytes(d)


def differenced(px, w, h, comps):
    """Horizontal differencing: what predictor 2 stores."""
    out = bytearray(px)
    stride = w * comps
    for y in range(h):
        for x in range(w - 1, 0, -1):
            for c in range(comps):
                i = y * stride + x * comps + c
                out[i] = (px[i] - px[i - comps]) & 0xFF
    return bytes(out)


def build(big, w, h, payload, strip_len, extra=()):
    """A minimal striped RGB TIFF in either byte order."""
    e = ">" if big else "<"
    tags = [
        (256, 3, 1, w),
        (257, 3, 1, h),
        (258, 3, 3, "BITS"),
        (259, 3, 1, 8 if extra else 1),
        (262, 3, 1, 2),
        (273, 4, 1, "STRIP"),
        (277, 3, 1, 3),
        (278, 3, 1, h),
        (279, 4, 1, strip_len),
    ] + list(extra)
    tags.sort(key=lambda t: t[0])
    n = len(tags)
    bits_at = 8 + 2 + n * 12 + 4
    strip_at = bits_at + 6
    body = struct.pack(e + "H", n)
    for tag, kind, count, val in tags:
        val = bits_at if val == "BITS" else (strip_at if val == "STRIP" else val)
        size = {1: 1, 3: 2, 4: 4}[kind]
        if tag == 258 or count * size > 4:
            payload_bytes = struct.pack(e + "I", val)
        elif kind == 3:
            payload_bytes = struct.pack(e + "H", val) + b"\0\0"
        else:
            payload_bytes = struct.pack(e + "I", val)
        body += struct.pack(e + "HHI", tag, kind, count) + payload_bytes
    body += struct.pack(e + "I", 0)
    head = (b"MM" if big else b"II") + struct.pack(e + "H", 42) + struct.pack(e + "I", 8)
    data = head + body
    assert len(data) == bits_at, (len(data), bits_at)
    data += struct.pack(e + "HHH", 8, 8, 8)
    return data + payload


def write(name, tif, raw):
    open(os.path.join(OUT, name + ".tif"), "wb").write(tif)
    if raw is not None:
        open(os.path.join(OUT, name + ".raw"), "wb").write(raw)


def main():
    for name, w, h, mode, comp in PILLOW_CASES:
        im = gradient(w, h, mode)
        im.save(os.path.join(OUT, name + ".tif"), "TIFF", compression=comp)
        write(name, open(os.path.join(OUT, name + ".tif"), "rb").read(), im.tobytes())
        print(f"{name:14s} {w}x{h} {mode:3s} {comp}")

    # Both byte orders, same image.
    px = flat(W, H, lambda x, y: ((x * 20) % 256, (y * 30) % 256, (x * y) % 256))
    for name, big in (("be_rgb", True), ("le_rgb", False)):
        write(name, build(big, W, H, px, len(px)), px)
        print(f"{name:14s} {W}x{H} RGB {'big' if big else 'little'}-endian, hand-built")

    # Predictor 2 over Deflate.
    ppx = flat(PW, PH, lambda x, y: ((x * 16) % 256, (y * 25) % 256, (x + y) % 256))
    comp = zlib.compress(differenced(ppx, PW, PH, 3), 9)
    write("pred_rgb", build(False, PW, PH, comp, len(comp), [(317, 3, 1, 2)]), ppx)
    print(f"{'pred_rgb':14s} {PW}x{PH} RGB deflate + predictor 2, hand-built")

    # Tiled: must be refused, so it needs no reference.
    write("tiled_rgb", build(False, PW, PH, ppx, len(ppx),
                             [(322, 3, 1, 16), (323, 3, 1, 16)]), None)
    print(f"{'tiled_rgb':14s} {PW}x{PH} RGB tiled — must be refused")

    # Every hand-built file read back through Pillow, so a fixture cannot be
    # wrong in the same direction as the decoder it tests.
    for name, want in (("be_rgb", px), ("le_rgb", px), ("pred_rgb", ppx)):
        got = Image.open(os.path.join(OUT, name + ".tif")).convert("RGB").tobytes()
        if got != want:
            raise SystemExit(f"{name}: Pillow disagrees with the fixture")
        print(f"  Pillow agrees: {name}")


if __name__ == "__main__":
    main()
