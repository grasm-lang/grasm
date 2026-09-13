#!/usr/bin/env bash
# Python pickles, protocols 0 to 5, against Python's own pickle.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/pickle_diff.py
