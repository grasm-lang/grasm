#!/usr/bin/env bash
# B61's packages against the references they reproduce. See scripts/b61_diff.py for the
# families and the reference each is held to.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in jq perl python3 rustc sbcl; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b61_diff.py
