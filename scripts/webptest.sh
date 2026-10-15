#!/usr/bin/env bash
# Does `packages/webp` agree with libwebp about the pixels?
#
# Lossless, so the bar is byte equality -- there is nothing for two correct
# decoders to disagree about. Ten fixtures, chosen for which parts of the
# format the encoder reaches for: the predictor and cross-colour transforms,
# colour indexing, the colour cache, backward references, single-pixel and
# single-row images, dimensions that are not a multiple of any tile size, and
# transparency.
#
# The references are read back from the files rather than kept from what was
# handed to the encoder, because lossless WebP zeroes the colour of fully
# transparent pixels. See scripts/fixtures/webp/make.py.
#
# A lossy file must be refused by name: VP8 is a video codec's intra frame and
# nothing here decodes it.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
FIX=scripts/fixtures/webp
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/webp-demo/src/main.gr --native -o "$work/dec" \
    >"$work/build.log" 2>&1; then
    echo "    the decoder did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

checked=0
for img in "$FIX"/*.webp; do
    name=$(basename "$img" .webp)
    # Checked below, as a refusal rather than a decode.
    [ "$name" = "lossy" ] && continue
    ref="$FIX/$name.rgba"
    [ -f "$ref" ] || { echo "    no reference for $name"; exit 1; }
    if ! "$work/dec" "$img" "$work/out.rgba" >"$work/dims" 2>&1; then
        echo "    $name did not decode: $(cat "$work/dims")"
        exit 1
    fi
    if ! cmp -s "$ref" "$work/out.rgba"; then
        echo "    $name does not match libwebp:"
        python3 -c "
ref = open('$ref','rb').read()
got = open('$work/out.rgba','rb').read()
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

# A lossy file carries a VP8 chunk and must be refused rather than guessed at.
if [ -f "$FIX/lossy.webp" ]; then
    if "$work/dec" "$FIX/lossy.webp" "$work/out.rgba" >/dev/null 2>&1; then
        echo "    a lossy file was decoded rather than refused"
        exit 1
    fi
    checked=$((checked + 1))
fi

echo "    images: $checked   exact against libwebp, and lossy refused"
[ "$checked" -ge 13 ]
