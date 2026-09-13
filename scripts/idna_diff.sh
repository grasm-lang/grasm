#!/usr/bin/env bash
# Internationalised domain names against Python's encodings.idna.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/idna_diff.py
