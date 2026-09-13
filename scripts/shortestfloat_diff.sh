#!/usr/bin/env bash
# The shortest round-tripping decimal against Python's repr.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/shortestfloat_diff.py
