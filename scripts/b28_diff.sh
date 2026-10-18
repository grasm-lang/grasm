#!/usr/bin/env bash
# Windows paths against Python's ntpath; txtar, httpproxy, bpf, the PROXY
# protocol and map tiles against Go modules from the module cache.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b28_diff.py
