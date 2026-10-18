#!/usr/bin/env bash
# go.mod files, signed notes, transparency logs and coverage profiles
# against golang.org/x/mod and x/tools.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b31_diff.py
