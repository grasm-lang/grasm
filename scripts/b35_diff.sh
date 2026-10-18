#!/usr/bin/env bash
# PBKDF2, password-based encryption, encrypted PKCS#8, PKCS#1, SEC1 and
# legacy PEM encryption against OpenSSL.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b35_diff.py
