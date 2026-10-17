#!/usr/bin/env bash
# goprintf (and the rest of B1's Go packages) against Go.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/gostd2_diff.py
