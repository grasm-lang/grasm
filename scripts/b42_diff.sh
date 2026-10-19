#!/usr/bin/env bash
# Mach-O symbols, universal files, code signatures, export tries, chained
# fixups, DWARF line tables, SQLite WAL and journal files, x86-64 lengths
# and Go tokens against nm, lipo, codesign, dyld_info, dwarfdump, SQLite,
# objdump and go/scanner.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in clang nm lipo libtool codesign dyld_info dwarfdump dsymutil objdump go; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b42_diff.py
