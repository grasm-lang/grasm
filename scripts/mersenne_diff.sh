#!/usr/bin/env bash
# MT19937 against Python's random, which is the same generator.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/mersenne_diff.py
