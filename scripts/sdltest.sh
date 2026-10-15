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

# The whole stack, not just the window: a JPEG decoded by `packages/jpeg`,
# composited by `packages/pcframe` with a half-transparent panel over it, and
# uploaded to the texture. Four packages, and if any of them stops agreeing
# with the others this is where it shows.
if ! "$GRASM" build packages/pcview-demo/src/main.gr --native --link SDL2 \
    -o "$work/pcview" >"$work/pcview.log" 2>&1; then
    echo "    the viewer did not build:"
    sed 's/^/      /' "$work/pcview.log"
    exit 1
fi
view=$(SDL_VIDEODRIVER=dummy "$work/pcview" scripts/fixtures/jpeg/g32_420.jpg 30 2>&1) || {
    echo "    the viewer did not run:"
    echo "$view" | sed 's/^/      /'
    exit 1
}
if [ "$view" != "32x24 3ch, 30 frames" ]; then
    echo "    expected: 32x24 3ch, 30 frames"
    echo "    got:      $view"
    exit 1
fi
echo "    jpeg -> pcframe -> texture: $view"

# The editor shell: seven packages, and a check that the palette *drew*
# rather than only that the loop ran. See scripts/lib/checkframe.py.
if ! "$GRASM" build packages/pcedit-demo/src/main.gr --native --link SDL2 \
    -o "$work/pcedit" >"$work/pcedit.log" 2>&1; then
    echo "    the editor did not build:"
    sed 's/^/      /' "$work/pcedit.log"
    exit 1
fi
edit=$(SDL_VIDEODRIVER=dummy "$work/pcedit" scripts/fixtures/jpeg/g80_ui.jpg \
    scripts/fixtures/font/shapes.ttf 5 "$work/frame.raw" 2>&1) || {
    echo "    the editor did not run:"
    echo "$edit" | sed 's/^/      /'
    exit 1
}
python3 scripts/lib/checkframe.py "$work/frame.raw" 80 240 150 || exit 1
echo "    editor shell: $edit"

# A frame budget rather than a bare timing, so this fails if the path becomes
# too slow to drive a window rather than merely getting slower.
#
# 10 ms against the ~0.4 ms measured idle. The margin is not politeness: this
# shares a machine with the other gates and read 4.55 ms under load, so a 5 ms
# budget would have failed for the load rather than for the code. A gate that
# flakes is a gate people learn to rerun.
python3 -c "
import sys
per = $per
if per > 10.0:
    print(f'    {per:.2f} ms a frame is past the 10 ms this gate allows')
    sys.exit(1)
"
