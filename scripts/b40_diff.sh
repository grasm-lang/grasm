#!/usr/bin/env bash
# Other languages' file formats (JSON Lines, JAR manifests, .pyc, TAP, V8
# and Storable serialization, Java serialization, class files, YAML)
# against Python, Java, Perl, Node and Ruby.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in java javac javap perl node ruby python3; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b40_diff.py
