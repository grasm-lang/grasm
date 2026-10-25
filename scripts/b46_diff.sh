#!/usr/bin/env bash
# Brace expansion, npm's INI, spdx-correct, npm-install-checks and
# npm-pick-manifest against npm's own copies; requirements files and entry
# points against pip 24.3 and Python 3.13's importlib.metadata.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in node npm python3.13; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
python3.13 -c "import pip._internal.req.req_file" 2>/dev/null || { echo "pip for python3.13 is required"; exit 1; }
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b46_diff.py
