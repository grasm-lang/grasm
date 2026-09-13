#!/usr/bin/env bash
# UTF-7, UTF-32, ISO-2022-KR and HZ against Python's codecs.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/encoding5_diff.py
