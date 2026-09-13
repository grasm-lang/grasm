#!/usr/bin/env bash
# Unified and context diffs against difflib.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/difftext_diff.py
