#!/usr/bin/env bash
# ecc, ecdh, ecdsa, dsa, rfc6979 and modarith against OpenSSL and Python.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/pubkey_diff.py
