#!/usr/bin/env bash
# mbox files against Python's own mailbox module.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/mbox_diff.py
