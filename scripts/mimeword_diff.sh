#!/usr/bin/env bash
# Encoded words against Python's own header decoder.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/mimeword_diff.py
