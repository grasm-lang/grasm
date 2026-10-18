#!/usr/bin/env bash
# Data structures against Python models.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/ds_diff.py
