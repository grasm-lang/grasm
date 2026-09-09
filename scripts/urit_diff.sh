#!/usr/bin/env bash
# URI templates against the working group's own test suite.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/urit_diff.py
