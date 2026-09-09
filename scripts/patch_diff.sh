#!/usr/bin/env bash
# JSON Patch and JSON Merge Patch against their specifications' own tests.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/patch_diff.py
