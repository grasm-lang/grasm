#!/usr/bin/env bash
# Package URLs against the purl-spec project's own test suite.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/purl_diff.py
