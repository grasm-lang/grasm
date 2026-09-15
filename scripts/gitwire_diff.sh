#!/usr/bin/env bash
# Ref names, colors, shortlog, raw diffs and bundles, against git.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/gitwire_diff.py
