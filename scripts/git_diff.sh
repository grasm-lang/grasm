#!/usr/bin/env bash
# .git/index, .gitattributes and .mailmap against git itself.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/git_diff.py
