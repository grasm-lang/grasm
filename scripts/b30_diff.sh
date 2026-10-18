#!/usr/bin/env bash
# ML-DSA and SLH-DSA against NIST's ACVP vectors and Cloudflare's circl.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b30_diff.py
