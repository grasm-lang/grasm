#!/usr/bin/env bash
# `audiodsp`'s filters, fades, gain and echo against ffmpeg's own.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/audio_diff.py
