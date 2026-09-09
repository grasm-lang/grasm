#!/usr/bin/env bash
# BIP-39 mnemonics and seeds against the reference implementation's vectors.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/bip39_diff.py
