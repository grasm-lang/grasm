#!/usr/bin/env bash
# md5crypt, apr1, sha256crypt and sha512crypt against openssl and the spec.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/unixcrypt_diff.py
