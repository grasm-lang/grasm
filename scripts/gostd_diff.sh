#!/usr/bin/env bash
# fnv, rc4, des, goquote, gotimefmt, tabwriter, textproto and suffixarray,
# against Go's standard library.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/gostd_diff.py
