#!/usr/bin/env bash
# Number packages against Python: brute force, fractions, struct, math, cmath.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/num_diff.py
