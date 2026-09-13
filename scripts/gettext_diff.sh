#!/usr/bin/env bash
# .mo catalogues and plural rules against Python's gettext.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/gettext_diff.py
