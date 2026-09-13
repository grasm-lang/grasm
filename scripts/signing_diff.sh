#!/usr/bin/env bash
# OpenSSH private keys, allowed_signers and commit messages, against the
# tools that own those formats.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/signing_diff.py
