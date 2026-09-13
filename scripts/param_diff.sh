#!/usr/bin/env bash
# RFC 2231 parameters, dbm.dumb databases and MIME base64, against Python.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/param_diff.py
