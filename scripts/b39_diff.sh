#!/usr/bin/env bash
# Audio and video formats (G.711, IMA ADPCM, .au, AIFF, CAF, YUV4MPEG2,
# ASS, FLAC, Ogg, Opus, Vorbis comments, ADTS, MPEG audio, MP4, Matroska)
# against FFmpeg, and afconvert where present.
set -uo pipefail
cd "$(dirname "$0")/.."
command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null || { echo "ffmpeg and ffprobe are required"; exit 1; }
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b39_diff.py
