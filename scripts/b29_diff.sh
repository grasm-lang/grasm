#!/usr/bin/env bash
# X448, Ed448, HPKE and age against Cloudflare's circl and metacubex/age.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b29_diff.py
