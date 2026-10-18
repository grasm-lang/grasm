#!/usr/bin/env bash
# SM4, Camellia, ARIA, SEED, Blowfish, CAST5, RC2 and IDEA in every mode
# against OpenSSL's libcrypto, and the cipher tables against their sources.
set -uo pipefail
cd "$(dirname "$0")/.."
python3 scripts/gen_cipher_tables.py --check || exit 1
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b33_diff.py
