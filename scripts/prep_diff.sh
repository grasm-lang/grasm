#!/usr/bin/env bash
# SASLprep, source-literal escapes and /etc account files, against Python.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/prep_diff.py
