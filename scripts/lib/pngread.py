#!/usr/bin/env python3
"""Reads the PNGs `packages/png` wrote and checks them against Pillow.

    python3 scripts/lib/pngread.py <dir>

Two separate things are asserted, and the second is the one that was
missing:

  * **the pixels survive** -- every file opens in Pillow and its samples are
    exactly what the writer was given, which is what "wrote a PNG" means;

  * **the bytes actually shrank** -- a compressible image must come out far
    smaller than its raw samples. `packages/png` wrote stored DEFLATE blocks
    for its whole life: valid PNGs that every reader accepts and that are
    *larger* than the pixels in them. A round-trip check passes on those,
    which is why one was not enough.

Noise is exempt from the second rule in the right direction: incompressible
data must *not* shrink, and a codec claiming otherwise is losing something.
"""

import glob
import os
import sys

from PIL import Image

# name -> (the most it may take as a fraction of raw, the least)
BOUNDS = {
    "flat": (0.10, 0.0),
    "gradient": (0.40, 0.0),
    "tiled": (0.10, 0.0),
    "grey": (0.60, 0.0),
    # Random bytes cannot be coded smaller. A little over 1.0 is the stream's
    # own overhead; far above it would mean stored blocks again.
    "noise": (1.15, 0.95),
}


def main():
    d = sys.argv[1]
    files = sorted(glob.glob(os.path.join(d, "*.png")))
    if not files:
        print("    no PNGs were written")
        return 1
    bad = 0
    checked = 0
    for path in files:
        name = os.path.basename(path)[:-4]
        try:
            im = Image.open(path)
            im.load()
        except Exception as e:
            print(f"    {name}: Pillow cannot read it: {e}")
            bad += 1
            continue
        raw = im.width * im.height * len(im.getbands())
        size = os.path.getsize(path)
        ratio = size / raw
        hi, lo = BOUNDS.get(name, (None, None))
        if hi is not None:
            if ratio > hi:
                print(f"    {name}: {size}B for {raw}B of pixels "
                      f"(ratio {ratio:.2f}, wanted at most {hi:.2f}) -- "
                      f"this is what stored blocks look like")
                bad += 1
            elif ratio < lo:
                print(f"    {name}: {size}B for {raw}B (ratio {ratio:.2f}, "
                      f"below {lo:.2f}) -- random data cannot compress, so "
                      f"something was dropped")
                bad += 1
        checked += 1
    print(f"    files: {checked}   read by Pillow, and the compressible ones "
          f"are under their size bound")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
