#!/usr/bin/env bash
# Where one character, and one word, ends — against UAX #29's own tests.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/segment_diff.py
