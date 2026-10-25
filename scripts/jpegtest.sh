#!/usr/bin/env bash
# Does `packages/jpeg` agree with libjpeg about the pixels?
#
# The decoder is held to a reference decoder rather than to a stored answer of
# its own, over 4:4:4, 4:2:2, 4:2:0, grayscale, dimensions that are not a
# multiple of the MCU, and a file with restart markers.
#
# Equality is not the bar and cannot be: the specification does not fix an
# inverse DCT, so two correct decoders differ slightly. The bar is a channel
# difference of at most 3 with a mean under 0.1, and the gate prints what it
# measured so a regression shows as a number rather than a pass or a fail.
#
# It is also what caught the upsampling. Box replication is what the
# specification describes; it is off by as much as 81 on a chroma edge against
# what every real decoder produces, because libjpeg filters 2x1 and 2x2 with a
# triangle filter. Without a reference decoder on the other side, that reads as
# a working JPEG decoder.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
FIX=scripts/fixtures/jpeg
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/jpeg-demo/src/main.gr --native -o "$work/dec" \
    >"$work/build.log" 2>&1; then
    echo "    the decoder did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

checked=0
worst=0
for jpg in "$FIX"/g*.jpg; do
    name=$(basename "$jpg" .jpg)
    ref="$FIX/$name.rgb"
    # The progressive fixture is here to be refused, not decoded.
    if [ "$name" = "g32_prog" ]; then
        if "$work/dec" "$jpg" "$work/out.raw" >/dev/null 2>&1; then
            echo "    a progressive file was decoded rather than refused"
            exit 1
        fi
        checked=$((checked + 1))
        continue
    fi
    [ -f "$ref" ] || { echo "    no reference for $name"; exit 1; }
    if ! "$work/dec" "$jpg" "$work/out.raw" >"$work/dims" 2>&1; then
        echo "    $name did not decode: $(cat "$work/dims")"
        exit 1
    fi
    d=$(python3 - "$ref" "$work/out.raw" <<'PY'
import sys
ref = open(sys.argv[1], "rb").read()
got = open(sys.argv[2], "rb").read()
if len(ref) != len(got):
    print(f"SIZE {len(ref)} vs {len(got)}")
    raise SystemExit
d = [abs(a - b) for a, b in zip(ref, got)]
print(f"{max(d)} {sum(d) / len(d):.4f}")
PY
)
    case "$d" in
        SIZE*) echo "    $name: $d"; exit 1 ;;
    esac
    mx=${d% *}
    mean=${d#* }
    if [ "$mx" -gt 3 ]; then
        echo "    $name: a channel is off by $mx, and 3 is the most an IDCT may differ by"
        exit 1
    fi
    python3 -c "
import sys
if $mean > 0.1:
    print('    $name: mean difference $mean is past 0.1')
    sys.exit(1)
"
    [ "$mx" -gt "$worst" ] && worst=$mx
    checked=$((checked + 1))
done

echo "    files: $checked   worst channel difference: $worst   (3 allowed)"
# A corpus that shrank is a gate that stopped checking (F76).
[ "$checked" -ge 7 ]
