#!/usr/bin/env python3
"""Reads the TIFFs `packages/tiff` wrote and checks them against Pillow.

    python3 scripts/lib/tiffread.py <dir>

The pixels are rebuilt here by the same rule the writer used, so what is
compared is the whole round trip through a reader that shares no code with
us -- not our decoder agreeing with our encoder.

Sizes are checked too, and the shapes are chosen to make that mean
something. PackBits codes runs of identical **bytes**, so an RGB flat fill
has nothing for it: 70,110,150 repeating never repeats a byte. The first set
of fixtures here was entirely RGB and PackBits compressed none of it, which
looked like a broken codec and was a broken choice of input. The
single-component shapes are the ones that exercise it.
"""

import glob
import os
import sys

from PIL import Image


def make(w, h, comps, kind):
    b = bytearray(w * h * comps)
    for i in range(w * h):
        for c in range(comps):
            if kind == "flat":
                v = 70 + c * 40
            elif kind == "runs":
                v = ((i // 17) % 3) * 90 + c
            elif kind == "gradient":
                v = (i % w) * 255 // w
            else:
                seed = ((i * 7 + c * 13 + 1) * 1103515245 + 12345) & 0xFFFFFFFFFFFFFFFF
                if seed >= 1 << 63:
                    seed -= 1 << 64
                v = ((seed & 2147483647) >> 16) & 255
            b[i * comps + c] = v & 255
    return bytes(b)


# shape -> the most PackBits may take, as a fraction of raw. Only the shapes
# with byte runs have one; the rest are expected not to shrink.
PACKBITS_BOUND = {"flat1": 0.30, "runs1": 0.40, "rgba": 0.85}

# Deflate works on the whole buffer, so the interleaved shapes are bounded
# too -- unlike PackBits, it finds the period.
DEFLATE_BOUNDED = ("flat1", "flat3", "runs1", "runs3")


def main():
    d = sys.argv[1]
    files = sorted(glob.glob(os.path.join(d, "*.tif")))
    if not files:
        print("    no TIFFs were written")
        return 1
    bad = 0
    checked = 0
    seen_bounds = set()
    for path in files:
        name = os.path.basename(path)[:-4]
        shape, comps, codec = name.rsplit("_", 2)
        comps = int(comps)
        try:
            im = Image.open(path)
            im.load()
        except Exception as e:
            print(f"    {name}: Pillow cannot read it: {e}")
            bad += 1
            continue
        kind = shape.rstrip("0123456789")
        if shape == "rgba":
            kind = "gradient"
        if shape == "one":
            kind = "flat"
        want = make(im.width, im.height, comps, kind)
        got = im.tobytes()
        if got != want:
            diff = sum(1 for a, b in zip(got, want) if a != b)
            print(f"    {name}: {im.size} {im.mode}, {diff} of {len(want)} "
                  f"bytes differ (got {len(got)} bytes, wanted {len(want)})")
            bad += 1
            continue
        raw = im.width * im.height * comps
        size = os.path.getsize(path)
        bound = PACKBITS_BOUND.get(shape)
        if codec == "32773" and bound is not None:
            seen_bounds.add(("packbits", shape))
            if size / raw > bound:
                print(f"    {name}: PackBits left {size}B for {raw}B of pixels "
                      f"(ratio {size/raw:.2f}, wanted at most {bound:.2f})")
                bad += 1
        if codec == "8" and shape in DEFLATE_BOUNDED:
            seen_bounds.add(("deflate", shape))
            if size / raw > 0.35:
                print(f"    {name}: Deflate left {size}B for {raw}B "
                      f"(ratio {size/raw:.2f})")
                bad += 1
        checked += 1
    # Derived from the tables above rather than counted to a number: adding
    # a bound without adding the shape that carries it would otherwise pass.
    want_bounds = {("packbits", k) for k in PACKBITS_BOUND}
    want_bounds |= {("deflate", k) for k in DEFLATE_BOUNDED}
    missing = want_bounds - seen_bounds
    if missing:
        for codec, shape in sorted(missing):
            print(f"    no {shape} file was written for {codec}, so its size "
                  f"bound checked nothing")
        return 1
    print(f"    files: {checked}   pixel-exact through Pillow, and "
          f"{len(seen_bounds)} of them under a size bound")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
