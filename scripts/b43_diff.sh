#!/usr/bin/env bash
# WHATWG URLs, UTS #46 domains, form data, MIME types, data: URLs,
# hosted-git-info, node-semver, npm-package-arg, negotiator and wheel tags
# against Node 25, npm's own modules and pip's packaging.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in node npm python3.13; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b43_diff.py
