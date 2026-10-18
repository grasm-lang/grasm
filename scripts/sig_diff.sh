#!/usr/bin/env bash
# Signal transforms, windows and splines against gonum and their definitions.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/sig_diff.py
