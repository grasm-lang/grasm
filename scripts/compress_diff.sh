#!/usr/bin/env bash
# Deflate, gzip and xz against Python's zlib, gzip and lzma.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/compress_diff.py
