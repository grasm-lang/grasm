#!/usr/bin/env bash
# cksum, openssl enc, ssh-keygen -L and infocmp, against the tools.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/unix_diff.py
