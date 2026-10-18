#!/usr/bin/env bash
# Package-ecosystem files against npm's own modules, Bundler, Go's x/mod and
# Python's importlib.metadata.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b18_diff.py
