#!/usr/bin/env bash
# git's index formats and pretty formats against git itself.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/git_diff.py
