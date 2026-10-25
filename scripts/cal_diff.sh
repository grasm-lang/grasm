#!/usr/bin/env bash
# Calendars against ICU, through Node's Intl.DateTimeFormat.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/cal_diff.py
