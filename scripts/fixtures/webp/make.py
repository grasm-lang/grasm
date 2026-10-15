#!/usr/bin/env python3
"""Build the lossless WebP fixtures `scripts/webptest.sh` decodes.

    python3 scripts/fixtures/webp/make.py

Needs Pillow. The output is committed, so this only runs when the cases
change.

The inputs are chosen for which parts of the format they make the encoder
reach for, which is not something a caller sets directly:

    noise     random bytes, so almost everything stays a literal
    gradient  smooth, so the predictor transform earns its place
    palette   few colours, so colour indexing packs several pixels a byte
    flat      one colour, so one literal and one long backward reference
    tiny      a single pixel, and a single row, and a single column
    odd       dimensions that are not a multiple of any tile size
    alpha     transparency, including fully transparent pixels
    cached    a small palette drawn at random, which fills the colour cache
    binalpha  random colour with alpha either 0 or 255, whose alpha channel
              has one symbol and so a prefix code that consumes no bits
    cachehit  positional colour with random alpha, which repeats colours
              often enough to produce colour-cache *hits*

The last two were not chosen by taste. Two bugs that cost real debugging --
a colour-cache hash constant off by one bit, and a single-symbol prefix code
read as one bit instead of none -- left every other fixture here passing. So
decoders were built with each bug and the fixtures searched for a file the
correct and broken ones disagree on. Asserting a gate covers something is not
the same as knowing it does.

Every reference is read **back from the file** rather than kept from what was
handed to the encoder. Lossless WebP zeroes the colour of fully transparent
pixels, so the two are not the same image, and a fixture that keeps the input
reports a decoder bug that is not there. That mistake was made here first and
cost an afternoon of looking in the wrong place.
"""

import os
import random

from PIL import Image

OUT = os.path.dirname(os.path.abspath(__file__))


def build(name, w, h, mode, fn):
    im = Image.new(mode, (w, h))
    px = im.load()
    for y in range(h):
        for x in range(w):
            px[x, y] = fn(x, y, w, h)
    path = os.path.join(OUT, name + ".webp")
    im.save(path, "WEBP", lossless=True)
    back = Image.open(path)
    open(os.path.join(OUT, name + ".rgba"), "wb").write(back.convert("RGBA").tobytes())
    # `scripts/imagetest.sh` compares the facade's RGB output for two of these.
    open(os.path.join(OUT, name + ".rgb"), "wb").write(back.convert("RGB").tobytes())
    print(f"{name:10s} {w:4d}x{h:<4d} {mode:5s} -> {os.path.getsize(path):5d} bytes")


def main():
    random.seed(77)
    pal = [(11, 22, 33), (200, 10, 90), (5, 240, 120), (250, 250, 0)]
    build("noise", 31, 17, "RGB",
          lambda x, y, w, h: (random.randrange(256), random.randrange(256), random.randrange(256)))
    build("gradient", 64, 48, "RGB",
          lambda x, y, w, h: (x * 255 // (w - 1), y * 255 // (h - 1), (x ^ y) % 256))
    build("palette", 40, 24, "RGB", lambda x, y, w, h: pal[(x // 3 + y // 2) % len(pal)])
    build("flat", 33, 9, "RGB", lambda x, y, w, h: (7, 99, 200))
    build("onepixel", 1, 1, "RGB", lambda x, y, w, h: (1, 2, 3))
    build("onerow", 64, 1, "RGB", lambda x, y, w, h: (x % 256, 0, 255 - x % 256))
    build("onecol", 1, 64, "RGB", lambda x, y, w, h: (0, y % 256, 128))
    build("odd", 17, 11, "RGB", lambda x, y, w, h: (x * 13 % 256, y * 7 % 256, 64))
    build("alpha", 24, 16, "RGBA",
          lambda x, y, w, h: (x * 9 % 256, y * 11 % 256, 70, 0 if (x + y) % 5 == 0 else 255))
    build("cached", 48, 32, "RGB", lambda x, y, w, h: pal[random.randrange(len(pal))])
    # The alpha channel takes two values, so its prefix code has one symbol
    # and consumes no bits; the colours repeat enough to produce cache hits.
    # Nothing else here reaches either path.
    build("binalpha", 31, 33, "RGBA",
          lambda x, y, w, h: (random.randrange(256), random.randrange(256),
                              random.randrange(256), random.choice([0, 255])))
    random.seed(77)
    build("cachehit", 5, 33, "RGBA",
          lambda x, y, w, h: (x % 256, y % 256, 128, random.randrange(256)))


if __name__ == "__main__":
    main()
