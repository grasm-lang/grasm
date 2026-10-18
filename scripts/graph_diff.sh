#!/usr/bin/env bash
# Graph algorithms against brute force.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/graph_diff.py
