#!/usr/bin/env bash
# XML-RPC calls, replies and faults against Python's xmlrpc.client.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/xmlrpc_diff.py
