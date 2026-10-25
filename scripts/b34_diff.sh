#!/usr/bin/env bash
# RIPEMD-160, Whirlpool, SM3, MDC-2, HMAC over thirteen hashes, and nine
# key derivation functions, against OpenSSL's libcrypto.
set -uo pipefail
cd "$(dirname "$0")/.."
python3 scripts/gen_hash_tables.py --check || exit 1
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b34_diff.py
