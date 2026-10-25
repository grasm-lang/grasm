#!/usr/bin/env bash
# B55's packages against the references they reproduce. See scripts/b55_diff.py for the
# families and the reference each is held to.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in java javac jq lua node python3.13 ruby; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b55_diff.py
