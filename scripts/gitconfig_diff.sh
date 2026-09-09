#!/usr/bin/env bash
# git config files, judged by git itself.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/gitconfig_diff.py
