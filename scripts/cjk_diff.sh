#!/usr/bin/env bash
# Johab, CP950, JIS X 0213 in four forms and EUC-JP with JIS X 0212,
# against Python's codecs.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/cjk_diff.py
