#!/usr/bin/env bash
# B54's packages against the references they reproduce. See scripts/b54_diff.py for the
# families and the reference each is held to.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in go java javac lua node perl python3.13 rustc; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b54_diff.py
