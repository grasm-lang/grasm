#!/usr/bin/env bash
# netrc files against Python's own netrc module.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/netrc_diff.py
