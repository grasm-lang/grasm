#!/usr/bin/env bash
# AES modes and MACs (CMAC, GMAC, KMAC, CCM, OCB, SIV, key wrap, XTS,
# GCM-SIV, CBC-CTS) against OpenSSL's libcrypto, in-process.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b32_diff.py
