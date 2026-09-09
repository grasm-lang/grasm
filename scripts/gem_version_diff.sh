#!/usr/bin/env bash
# RubyGems version ordering against Ruby's own Gem::Version.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/gem_version_diff.py
