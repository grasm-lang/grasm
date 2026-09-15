#!/usr/bin/env bash
# CP932, CP949, Big5-HKSCS and EBCDIC against Python's codecs.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/dbcs_diff.py
