#!/usr/bin/env bash
# A RESP server, judged by a client that speaks the protocol from its rules.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/resp_diff.py
