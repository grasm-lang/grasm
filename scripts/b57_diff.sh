#!/usr/bin/env bash
# B57's packages against the references they reproduce, all CPython 3.13. See scripts/b57_diff.py for the
# families and the reference each is held to.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in python3.13; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b57_diff.py
