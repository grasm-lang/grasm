#!/usr/bin/env bash
# Patch ids against git patch-id, SSH signatures against ssh-keygen.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/gitsign_diff.py
