#!/usr/bin/env python3
"""Did the editor's palette actually draw, or did the loop only run?

A blank panel is one colour. A drawn one carries the theme's raised, accent
and track colours too, because only widgets paint those. Called by
`scripts/sdltest.sh` with the frame `pcedit-demo` wrote.

    checkframe.py <frame.raw> <image_width> <height> <palette_width>
"""

import collections
import sys


def main():
    path, iw, h, pal = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
    w = iw + pal
    d = open(path, "rb").read()
    if len(d) != w * h * 4:
        print(f"    frame is {len(d)} bytes, expected {w * h * 4}")
        return 1

    def px(x, y):
        i = (y * w + x) * 4
        return tuple(d[i : i + 3])

    cols = collections.Counter(px(x, y) for y in range(h) for x in range(iw, w))
    # `pcui.dark()`'s own colours. A panel nobody drew on has only the first.
    need = {(24, 26, 32): "panel", (44, 48, 58): "raised", (26, 77, 255): "accent"}
    missing = [name for c, name in need.items() if cols[c] == 0]
    if missing:
        print(f"    the palette did not draw: no {', '.join(missing)}")
        print(f"    colours present: {cols.most_common(5)}")
        return 1
    if len(cols) < 4:
        print(f"    the palette has {len(cols)} colours, so nothing drew over it")
        return 1
    print(f"    palette drew: {len(cols)} colours, accent on {cols[(26, 77, 255)]} pixels")
    return 0


if __name__ == "__main__":
    sys.exit(main())
