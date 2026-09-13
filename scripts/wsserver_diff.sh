#!/usr/bin/env bash
# A WebSocket server holding many conversations on one thread.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/wsserver_diff.py
