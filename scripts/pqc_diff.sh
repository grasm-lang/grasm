#!/usr/bin/env bash
# The post-quantum packages against OpenSSL.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/pqc_diff.py
