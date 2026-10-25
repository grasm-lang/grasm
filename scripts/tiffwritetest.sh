#!/usr/bin/env bash
# Does `packages/tiff` write TIFFs that Pillow reads back unchanged?
#
# Eighteen files: six shapes by three codecs (uncompressed, Deflate,
# PackBits). The pixels are rebuilt independently in
# `scripts/lib/tiffread.py` and compared there, so this never asks our own
# decoder whether our own encoder was right.
#
# Sizes are bounded as well as pixels, because a codec that quietly does
# nothing still round-trips. That check is only worth having over input the
# codec can actually work on -- see the note in tiffread.py about the first
# set of fixtures, which was all RGB and gave PackBits nothing to find.
#
# LZW is read by this package and not written. `lzw` can code it, but the
# bit packing has enough corners that shipping a writer without an
# independent reader to check it against is how a subtly wrong file escapes.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! python3 -c "import PIL" 2>/dev/null; then
    echo "    Pillow is not installed, so nothing was checked:"
    echo "      python3 -m pip install pillow"
    exit 1
fi

if ! "$GRASM" build packages/tiffwrite-demo/src/main.gr --native -o "$work/w" \
    >"$work/build.log" 2>&1; then
    echo "    the writer did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

mkdir -p "$work/out"
if ! "$work/w" "$work/out" >"$work/wrote.txt" 2>&1; then
    echo "    the writer did not run:"
    sed 's/^/      /' "$work/wrote.txt"
    exit 1
fi

python3 scripts/lib/tiffread.py "$work/out" || exit 1

wrote=$(grep -c . "$work/wrote.txt")
found=$(find "$work/out" -name '*.tif' | wc -l | tr -d ' ')
[ "$wrote" = "$found" ] || {
    echo "    the writer reported $wrote images and left $found on disk"
    exit 1
}
echo "    images: $found   every one reported was written"
