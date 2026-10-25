#!/usr/bin/env bash
# Does `packages/tiff` agree with Pillow about the pixels?
#
# TIFF is lossless, so the bar is byte equality rather than a tolerance --
# unlike JPEG, where two correct decoders differ because the specification
# fixes no inverse DCT. Anything short of exact here is a defect.
#
# Ten files: raw, LZW, Deflate and PackBits; RGB and grayscale; both byte
# orders; horizontal differencing; dimensions that are not a round number; and
# a tiled file that must be refused rather than decoded wrongly.
#
# The two hand-built files exist because Pillow will not write them. It emits
# no big-endian TIFF and no predictor, and those are the two paths most likely
# to be wrong in a decoder nobody tested them against -- `be_rgb` and
# `pred_rgb` are built by `scripts/fixtures/tiff/make.py`, which checks each
# one back through Pillow before writing it.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
FIX=scripts/fixtures/tiff
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/tiff-demo/src/main.gr --native -o "$work/dec" \
    >"$work/build.log" 2>&1; then
    echo "    the decoder did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

checked=0
for tif in "$FIX"/*.tif; do
    name=$(basename "$tif" .tif)
    if [ "$name" = "tiled_rgb" ]; then
        if "$work/dec" "$tif" "$work/out.raw" >/dev/null 2>&1; then
            echo "    a tiled file was decoded rather than refused"
            exit 1
        fi
        checked=$((checked + 1))
        continue
    fi
    ref="$FIX/$name.raw"
    [ -f "$ref" ] || { echo "    no reference for $name"; exit 1; }
    if ! "$work/dec" "$tif" "$work/out.raw" >"$work/dims" 2>&1; then
        echo "    $name did not decode: $(cat "$work/dims")"
        exit 1
    fi
    if ! cmp -s "$ref" "$work/out.raw"; then
        echo "    $name does not match Pillow byte for byte:"
        python3 -c "
ref = open('$ref','rb').read()
got = open('$work/out.raw','rb').read()
if len(ref) != len(got):
    print(f'      size {len(ref)} vs {len(got)}')
else:
    d = [i for i, (a, b) in enumerate(zip(ref, got)) if a != b]
    print(f'      {len(d)} of {len(ref)} bytes differ, first at {d[0]}')
"
        exit 1
    fi
    checked=$((checked + 1))
done

echo "    files: $checked   exact: $((checked - 1))   refused: 1 (tiled)"
# A corpus that shrank is a gate that stopped checking (F76).
[ "$checked" -ge 11 ]
