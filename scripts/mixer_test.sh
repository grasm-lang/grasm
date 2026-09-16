#!/usr/bin/env bash
# The editor, checked from outside itself.
#
# Five things a self-test inside the program cannot check for itself:
#
#   1. that all three engines agree on it — and on what the editor *draws*,
#      which is why `mixer screen` prints a frame instead of sending one
#   2. that a render is deterministic: the same project twice, byte for byte
#   3. that what it writes is a WAV other software accepts, asked of ffprobe
#   4. that the audio is right, asked of ffmpeg: one file through one track is
#      that file at the centre of the stereo field, and equal-power panning
#      says exactly what that means
#   5. that the interactive half works at all: a pseudo-terminal is allocated,
#      raw mode is entered for real, keys are typed, and the file the editor
#      writes has to show the edit
#
# The fourth is the point. A mixdown that is internally consistent and
# 3 dB off is exactly what a self-test written by the same hand would miss.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

SRC=apps/mixer/src/main.gr
WORK="${TMPDIR:-/tmp}/grasm_mixer_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

pass=0
fail=0
declare -a FAILED=()

ok() {
    pass=$((pass + 1))
    echo "  ok    $1"
}

bad() {
    fail=$((fail + 1))
    FAILED+=("$1")
    echo "  FAIL  $1"
    if [ $# -gt 1 ]; then
        printf '        %s\n' "$2"
    fi
}

echo "mixer: building"
if ! "$GRASM" build "$SRC" --native -o "$WORK/mixer" >"$WORK/build.log" 2>&1; then
    echo "  BUILD FAILED (native)"
    sed -n '1,20p' "$WORK/build.log"
    exit 1
fi
if ! "$GRASM" build "$SRC" -o "$WORK/mixer_bundled" >>"$WORK/build.log" 2>&1; then
    echo "  BUILD FAILED (bundled)"
    sed -n '1,20p' "$WORK/build.log"
    exit 1
fi
M="$WORK/mixer"

echo "mixer: engines"
"$GRASM" run "$SRC" -- selftest >"$WORK/st_interp.out" 2>&1
interp=$?
"$M" selftest >"$WORK/st_native.out" 2>&1
native=$?
"$WORK/mixer_bundled" selftest >"$WORK/st_bundled.out" 2>&1
bundled=$?
if [ "$interp" -eq 0 ]; then
    ok "the self-test holds under the interpreter"
else
    bad "the self-test holds under the interpreter" "$(tail -3 "$WORK/st_interp.out")"
fi
if diff -q "$WORK/st_interp.out" "$WORK/st_native.out" >/dev/null &&
    [ "$interp" -eq "$native" ]; then
    ok "the native build says the same thing"
else
    bad "the native build says the same thing" \
        "$(diff "$WORK/st_interp.out" "$WORK/st_native.out" | head -6)"
fi
if diff -q "$WORK/st_interp.out" "$WORK/st_bundled.out" >/dev/null &&
    [ "$interp" -eq "$bundled" ]; then
    ok "and so does the bundled one"
else
    bad "and so does the bundled one" \
        "$(diff "$WORK/st_interp.out" "$WORK/st_bundled.out" | head -6)"
fi

# The drawing is a pure function of the project, so every engine draws the
# same frame — including the waveform, which is the rendered audio.
"$GRASM" run "$SRC" -- screen >"$WORK/screen_interp.out" 2>&1
"$M" screen >"$WORK/screen_native.out" 2>&1
if diff -q "$WORK/screen_interp.out" "$WORK/screen_native.out" >/dev/null; then
    ok "and draws the same frame, waveform and all"
else
    bad "and draws the same frame, waveform and all" \
        "$(diff "$WORK/screen_interp.out" "$WORK/screen_native.out" | head -6)"
fi
if grep -q "bass" "$WORK/screen_native.out" &&
    grep -q "space play" "$WORK/screen_native.out"; then
    ok "the frame has the tracks and the keys on it"
else
    bad "the frame has the tracks and the keys on it" \
        "$(head -4 "$WORK/screen_native.out")"
fi

echo "mixer: rendering"
(cd "$WORK" && "$M" render once.wav >render1.out 2>&1)
(cd "$WORK" && "$M" render twice.wav >render2.out 2>&1)
if [ -s "$WORK/once.wav" ] && cmp -s "$WORK/once.wav" "$WORK/twice.wav"; then
    ok "the same project renders byte for byte the same file"
else
    bad "the same project renders byte for byte the same file" \
        "$(cat "$WORK/render1.out")"
fi

if command -v ffprobe >/dev/null 2>&1; then
    shape=$(ffprobe -v error -select_streams a:0 \
        -show_entries stream=channels,sample_rate,bits_per_raw_sample \
        -show_entries format=duration -of csv=p=0:nk=1 "$WORK/once.wav" |
        tr '\n' ' ')
    case "$shape" in
        *44100*) ok "ffprobe reads it as a 44.1 kHz stereo file ($shape)" ;;
        *) bad "ffprobe reads it as a 44.1 kHz stereo file" "$shape" ;;
    esac
else
    echo "  skip  ffprobe is not installed"
fi

echo "mixer: against ffmpeg"
if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "  skip  ffmpeg is not installed"
else
    # One mono file, one track, no gain and no pan: the mix is that file at
    # the centre, which under equal-power panning is 1/sqrt(2) in each ear.
    ffmpeg -v error -f lavfi \
        -i "sine=frequency=330:duration=1:sample_rate=44100" \
        -f lavfi -i "anoisesrc=d=1:c=pink:r=44100:a=0.3:seed=11" \
        -filter_complex amix=inputs=2 -c:a pcm_s16le -y "$WORK/one.wav" \
        >/dev/null 2>&1
    (cd "$WORK" && "$M" render mine.wav one.wav >mine.out 2>&1)
    ffmpeg -v error -i "$WORK/one.wav" -af \
        "aformat=sample_fmts=dblp,pan=stereo|c0=0.7071067811865476*c0|c1=0.7071067811865476*c0" \
        -c:a pcm_s16le -y "$WORK/theirs.wav" >/dev/null 2>&1
    gap=$(python3 - "$WORK/mine.wav" "$WORK/theirs.wav" <<'PY'
import struct
import sys
import wave


def read(path):
    with wave.open(path, "rb") as w:
        return w.getnchannels(), w.readframes(w.getnframes())


ch_a, a = read(sys.argv[1])
ch_b, b = read(sys.argv[2])
n = min(len(a), len(b)) // 2
va = struct.unpack_from("<%dh" % n, a)
vb = struct.unpack_from("<%dh" % n, b)
worst = max(abs(x - y) for x, y in zip(va, vb)) if n else 9999
print("%d %d %d %d" % (worst, ch_a, ch_b, n))
PY
)
    set -- $gap
    worst=${1:-9999}
    channels=${2:-0}
    # One 16-bit step: both sides round a float to an integer sample, and
    # they are allowed to land either side of a tie.
    if [ "$worst" -le 1 ] && [ "$channels" -eq 2 ]; then
        ok "a file through one track matches ffmpeg's own pan (worst $worst of 32768)"
    else
        bad "a file through one track matches ffmpeg's own pan" \
            "worst sample $worst, channels $channels"
    fi
fi

# 5. The editor itself, driven by keys through a pseudo-terminal.
#
# Everything above tests the program with a person taken out of it. This puts
# one back: a terminal is allocated, raw mode is entered for real, keys are
# typed, and the file the editor writes has to show the edit. Without this the
# whole interactive half is untested.
echo "mixer: at the keyboard"
cat >"$WORK/drive.py" <<'DRIVER'
import os
import pty
import re
import select
import sys
import threading
import time

prog, keys = sys.argv[1], sys.argv[2]
pid, fd = pty.fork()
if pid == 0:
    os.execv(prog, [prog])

# Read continuously. A full-screen repaint is larger than a pty's buffer, so
# a driver that types first and reads afterwards deadlocks against the program
# it is driving.
chunks = []
stop = False


def reader():
    while not stop:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if not ready:
            continue
        try:
            data = os.read(fd, 65536)
        except OSError:
            return
        if not data:
            return
        chunks.append(data)


thread = threading.Thread(target=reader, daemon=True)
thread.start()
time.sleep(1.0)
for key in keys:
    os.write(fd, key.encode())
    time.sleep(0.35)
time.sleep(1.0)
stop = True
try:
    os.waitpid(pid, os.WNOHANG)
except ChildProcessError:
    pass
text = b"".join(chunks).decode("utf-8", "replace")
sys.stdout.write(re.sub(r"\x1b\[[0-9;?]*[a-zA-Z]", "", text))
DRIVER

if ! command -v python3 >/dev/null 2>&1; then
    echo "  skip  python3 is not installed, so no pseudo-terminal"
else
    typed="$WORK/typed"
    mkdir -p "$typed"
    # j selects the second track, m mutes it, w writes, q quits.
    (cd "$typed" && timeout 90 python3 "$WORK/drive.py" "$M" 'jmwq' \
        >"$WORK/typed.out" 2>&1)
    (cd "$typed" && "$M" render unmuted.wav >/dev/null 2>&1)
    if grep -q "bass" "$WORK/typed.out"; then
        ok "the editor paints a frame on a real terminal"
    else
        bad "the editor paints a frame on a real terminal" \
            "$(head -c 200 "$WORK/typed.out")"
    fi
    if [ -s "$typed/mixdown.wav" ]; then
        ok "and w writes the mixdown it is showing"
    else
        bad "and w writes the mixdown it is showing" "no file was written"
    fi
    if [ -s "$typed/mixdown.wav" ] &&
        ! cmp -s "$typed/mixdown.wav" "$typed/unmuted.wav"; then
        ok "and the track muted at the keyboard is missing from it"
    else
        bad "and the track muted at the keyboard is missing from it" \
            "the file is the same as an unmuted render"
    fi
fi

echo
echo "mixer_test: $pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then
    for name in "${FAILED[@]}"; do
        echo "  - $name"
    done
    exit 1
fi
