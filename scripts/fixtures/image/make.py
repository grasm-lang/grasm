#!/usr/bin/env python3
"""Build the PNG fixtures `scripts/imagetest.sh` uses.

    python3 scripts/fixtures/image/make.py

Needs Pillow. The JPEG and TIFF cases live under their own decoders'
fixtures; these are the PNG colour types, because those are what
`packages/image` has to normalise: a palette index becomes the RGB triple it
names, grayscale is replicated across three channels, and alpha is dropped.

The reference is RGB in every case, which is what `image.to_rgb` promises
whatever the file carried.
"""

import os

from PIL import Image

OUT = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(OUT, "..", "jpeg", "g32_420.jpg")


def main():
    base = Image.open(SRC).convert("RGB")
    cases = [
        ("rgb", base),
        ("palette", base.convert("P", palette=Image.ADAPTIVE, colors=64)),
        ("gray", base.convert("L")),
        ("rgba", base.convert("RGBA")),
    ]
    for name, im in cases:
        path = os.path.join(OUT, f"{name}.png")
        im.save(path)
        # Read back rather than converting in memory: the reference must be
        # what the file decodes to, not what was handed to the encoder.
        rgb = Image.open(path).convert("RGB").tobytes()
        open(os.path.join(OUT, f"{name}.rgb"), "wb").write(rgb)
        print(f"{name:9s} {im.mode:5s} {im.size} -> {len(rgb)} bytes of RGB")


if __name__ == "__main__":
    main()
