#!/usr/bin/env bash
# Country codes, names and zones against the tz database's own tables.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/iso3166_diff.py
