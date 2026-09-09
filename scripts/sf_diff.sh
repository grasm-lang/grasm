#!/usr/bin/env bash
# HTTP structured fields against the working group's own test suite.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/sf_diff.py
