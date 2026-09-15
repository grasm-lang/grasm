#!/usr/bin/env bash
# getopt, float arithmetic, Python literals, marshal and pprint, against Python.
set -uo pipefail
cd "$(dirname "$0")/.."
# The corpus builds sets of strings, whose order follows the hash seed.
PYTHONHASHSEED=0 GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/pydata_diff.py
