#!/usr/bin/env bash
# MD5, HMAC-MD5 and UUID v3/v5 against Python's hashlib and uuid.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/md5_diff.py
