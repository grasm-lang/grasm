#!/usr/bin/env bash
# Address headers against Python's own address parser.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/address_diff.py
