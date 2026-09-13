#!/usr/bin/env bash
# Suggestion ranking against difflib, auto-junk cutoff included.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/closematch_diff.py
