#!/usr/bin/env bash
# Does `packages/png` write PNGs that something other than itself can read,
# and does it actually compress them?
#
# `packages/png` had no tests at all and only its *decode* path was gated, by
# `imagetest.sh`. That is how `zlib.compress_stored` survived in the writer:
# stored DEFLATE blocks are valid, every reader accepts them, and a round
# trip through our own decoder passes -- while every PNG written came out
# larger than the pixels in it. A decoder agreeing with the encoder that
# wrote the file proves neither one right.
#
# So two things are checked, against Pillow rather than against us: the
# pixels survive, and the bytes shrink. See scripts/lib/pngread.py for the
# bounds and why noise has one in the other direction.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! python3 -c "import PIL" 2>/dev/null; then
    # A skip is not a pass: `allgates.sh` reads the exit code.
    echo "    Pillow is not installed, so nothing was checked:"
    echo "      python3 -m pip install pillow"
    exit 1
fi

if ! "$GRASM" build packages/pngwrite-demo/src/main.gr --native -o "$work/w" \
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

python3 scripts/lib/pngread.py "$work/out" || exit 1

# Every image the writer reported must be on disk. A writer that printed a
# line and wrote nothing would otherwise pass on the files it did write.
wrote=$(grep -c . "$work/wrote.txt")
found=$(find "$work/out" -name '*.png' | wc -l | tr -d ' ')
if [ "$wrote" != "$found" ]; then
    echo "    the writer reported $wrote images and left $found on disk"
    exit 1
fi
echo "    images: $found   every one reported was written"
