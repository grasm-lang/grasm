#!/usr/bin/env bash
# S2, CRC-32C, NAT-PMP and Canonical XML against klauspost/compress,
# hash/crc32, go-nat-pmp and xmllint.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b28c_diff.py
