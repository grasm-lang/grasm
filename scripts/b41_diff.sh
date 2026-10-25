#!/usr/bin/env bash
# Archives (ZipCrypto, shar, .deb, xar, BOM) and git's inner formats
# (reftable, bitmaps, notes, ignore rules, status) against zip, sh, ar,
# tar, xar, lsbom and git.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in git zip unzip shar ar tar xar mkbom lsbom zstd; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b41_diff.py
