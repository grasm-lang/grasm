#!/usr/bin/env bash
# Digest files against what shasum writes.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/checksum_diff.py
