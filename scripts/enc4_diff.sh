#!/usr/bin/env bash
# base32hex, stat.filemode and html.unescape against Python.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/enc4_diff.py
