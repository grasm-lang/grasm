#!/usr/bin/env bash
# B47's packages against the references they reproduce. See scripts/b47_diff.py for the
# families and the reference each is held to.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in curl go java javac lz4 node npm python3.13; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b47_diff.py
