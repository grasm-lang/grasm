#!/usr/bin/env bash
# Does the GUI path work end to end?
#
# Generate a frame into a mutable byte buffer, upload it to a streaming
# texture, clear, copy, present, and poll for events -- through real SDL2,
# with no display, because `SDL_VIDEODRIVER=dummy` makes every call work
# without one. So this is not a compile check: the window, the renderer and
# the texture are really created and the frames really go through.
#
# It answers the question `packages/sdl` exists to answer. The claim was that
# binding SDL2 before the pixel pipeline got faster "gets a window that shows
# a frame every few seconds". It does not: the measurement below is the whole
# path, and it is under a millisecond a frame.
#
# SDL2 is a system library and is not vendored. If it is absent this says so
# and stops, rather than passing -- a gate that did not run is not a gate that
# passed (F76).
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Homebrew's lib directory is not on the linker's default search path, and
# `clang` reads LIBRARY_PATH. A prefix from `brew` rather than a literal, so
# this works on both Apple-silicon and Intel layouts.
if [ -z "${LIBRARY_PATH:-}" ] && command -v brew >/dev/null 2>&1; then
    LIBRARY_PATH="$(brew --prefix)/lib"
    export LIBRARY_PATH
fi

found=""
for d in ${LIBRARY_PATH:-} /usr/local/lib /usr/lib; do
    for ext in dylib so; do
        [ -e "$d/libSDL2.$ext" ] && found="$d/libSDL2.$ext" && break 2
    done
done
if [ -z "$found" ]; then
    echo "    SDL2 is not installed, so the GUI path was not checked."
    echo "    install it (brew install sdl2) and run this again."
    exit 1
fi
echo "    linking $found"

if ! "$GRASM" build packages/sdl-demo/src/main.gr --native --link SDL2 \
    -o "$work/sdldemo" >"$work/build.log" 2>&1; then
    echo "    the demo did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

frames=600
start=$(python3 -c 'import time; print(time.time())')
out=$(SDL_VIDEODRIVER=dummy "$work/sdldemo" "$frames" 2>&1) || {
    echo "    the demo did not run:"
    echo "$out" | sed 's/^/      /'
    exit 1
}
end=$(python3 -c 'import time; print(time.time())')

# The program prints what it drew, and the count is checked rather than
# trusted: a loop that exited early would otherwise read as a pass.
want="drew $frames frames of 256x256 RGBA"
if [ "$out" != "$want" ]; then
    echo "    expected: $want"
    echo "    got:      $out"
    exit 1
fi

per=$(python3 -c "print(f'{($end - $start) * 1000 / $frames:.2f}')")
echo "    frames: $frames   per frame: ${per} ms   (16.67 ms is 60 fps)"

# A frame budget rather than a bare timing, so this fails if the path becomes
# too slow to drive a window rather than merely getting slower. Generous
# against the ~0.4 ms measured, because this shares a machine with the other
# gates.
python3 -c "
import sys
per = $per
if per > 5.0:
    print(f'    {per:.2f} ms a frame is past the 5 ms this gate allows')
    sys.exit(1)
"
