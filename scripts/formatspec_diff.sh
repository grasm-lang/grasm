#!/usr/bin/env bash
# The format mini-language against Python's format().
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/formatspec_diff.py
