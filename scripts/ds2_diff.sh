#!/usr/bin/env bash
# Caches, consistent hashing, MinHash and filters against Python models and Go.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/ds2_diff.py
