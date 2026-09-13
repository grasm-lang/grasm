#!/usr/bin/env bash
# Public keys and fingerprints against ssh-keygen.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/sshkey_diff.py
