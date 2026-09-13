#!/usr/bin/env bash
# packed-refs, reflogs, .gitmodules and git's date formats, against git.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/repo_diff.py
