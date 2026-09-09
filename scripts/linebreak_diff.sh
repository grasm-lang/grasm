#!/usr/bin/env bash
# Line break opportunities against UAX #14's own test file.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/linebreak_diff.py
