#!/usr/bin/env bash
# EUC-JP and ISO-2022-JP against Python's codecs.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/japanese_diff.py
