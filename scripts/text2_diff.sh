#!/usr/bin/env bash
# Z85, string.Template, expandvars, format strings and ndiff, against Python.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/text2_diff.py
