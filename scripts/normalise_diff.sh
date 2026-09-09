#!/usr/bin/env bash
# NFC, NFD, NFKC and NFKD against the Unicode Consortium's own test file.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/normalise_diff.py
