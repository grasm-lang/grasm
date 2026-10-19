#!/usr/bin/env bash
# CFF, GPOS, GDEF, HVAR/VVAR/MVAR, gvar, trak, feat, meta/ltag,
# vmtx/VORG and BASE against fontTools on the system's fonts.
set -uo pipefail
cd "$(dirname "$0")/.."
python3 -c "import fontTools" 2>/dev/null || { echo "fontTools is required"; exit 1; }
[ -d /System/Library/Fonts ] || { echo "the system fonts are required"; exit 1; }
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b44_diff.py
