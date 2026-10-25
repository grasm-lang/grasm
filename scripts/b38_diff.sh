#!/usr/bin/env bash
# OpenType tables (name, OS/2, post, kern, fvar, avar, STAT, COLR, CPAL,
# GSUB) and font collections against fontTools.
set -uo pipefail
cd "$(dirname "$0")/.."
python3 scripts/gen_font_tables.py --check || exit 1
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b38_diff.py
