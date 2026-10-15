#!/usr/bin/env bash
# Does `packages/image` dispatch and normalise correctly?
#
# The facade's job is that a caller never asks which decoder to call, and
# never sees six PNG colour types, a palette to resolve or a 16-bit sample.
# So this checks the normalisation rather than the decoders, which have gates
# of their own: every file comes back as RGB matching Pillow byte for byte.
#
# JPEG is in the list for the dispatch rather than the pixels -- it is the one
# format where exact equality is not available, and `scripts/jpegtest.sh`
# holds it to libjpeg instead.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/image-demo/src/main.gr --native -o "$work/dec" \
    >"$work/build.log" 2>&1; then
    echo "    the facade did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

checked=0
for pair in \
    "scripts/fixtures/image/rgb.png:scripts/fixtures/image/rgb.rgb:png" \
    "scripts/fixtures/image/palette.png:scripts/fixtures/image/palette.rgb:png" \
    "scripts/fixtures/image/gray.png:scripts/fixtures/image/gray.rgb:png" \
    "scripts/fixtures/image/rgba.png:scripts/fixtures/image/rgba.rgb:png" \
    "scripts/fixtures/tiff/raw_rgb.tif:scripts/fixtures/tiff/raw_rgb.raw:tiff" \
    "scripts/fixtures/tiff/be_rgb.tif:scripts/fixtures/tiff/be_rgb.raw:tiff"
do
    src=${pair%%:*}
    rest=${pair#*:}
    ref=${rest%%:*}
    want=${rest#*:}
    out=$("$work/dec" "$src" "$work/out.raw" 2>&1) || {
        echo "    $src did not decode: $out"
        exit 1
    }
    got_fmt=${out%% *}
    if [ "$got_fmt" != "$want" ]; then
        echo "    $src sniffed as $got_fmt, expected $want"
        exit 1
    fi
    if ! cmp -s "$ref" "$work/out.raw"; then
        echo "    $src does not match Pillow as RGB"
        exit 1
    fi
    checked=$((checked + 1))
done

# Dispatch for the format whose pixels are checked elsewhere.
jf=$("$work/dec" scripts/fixtures/jpeg/g32_420.jpg "$work/out.raw" 2>&1)
case "$jf" in
    "jpeg 32 24 3") checked=$((checked + 1)) ;;
    *) echo "    a JPEG sniffed as: $jf"; exit 1 ;;
esac

# A file of no known format must be refused, not guessed at.
if "$work/dec" scripts/fixtures/font/shapes.ttf "$work/out.raw" >/dev/null 2>&1; then
    echo "    a TrueType font was decoded as an image"
    exit 1
fi
checked=$((checked + 1))

echo "    files: $checked   formats: png (4 colour types), tiff, jpeg, and one refused"
[ "$checked" -ge 8 ]
