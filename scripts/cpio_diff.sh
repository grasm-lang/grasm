#!/usr/bin/env bash
# cpio archives against cpio(1) itself, both directions.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/cpio_diff.py
