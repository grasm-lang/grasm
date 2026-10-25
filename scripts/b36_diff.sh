#!/usr/bin/env bash
# PKCS#12 files against OpenSSL and Java KeyStores against keytool.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b36_diff.py
