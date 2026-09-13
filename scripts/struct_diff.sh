#!/usr/bin/env bash
# Fixed binary records against Python's own `struct`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/struct_diff.py
