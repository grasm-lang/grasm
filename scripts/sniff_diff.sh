#!/usr/bin/env bash
# What a file is, against file(1)'s own answer.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/sniff_diff.py
