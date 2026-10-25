#!/usr/bin/env bash
# Rebase and bind opcodes, weak binds, build versions, code-signing
# requirements, archive indexes, .lzma, PackBits, DWARF 5's address,
# string-offset, range-list and name tables, and compact unwind info
# against dyld_info, objdump, vtool, csreq, codesign, nm, Python's lzma,
# libtiff (through Pillow) and dwarfdump.
set -uo pipefail
cd "$(dirname "$0")/.."
for t in clang clang++ dyld_info objdump vtool csreq codesign lipo nm libtool ar otool dwarfdump dsymutil; do
  command -v "$t" >/dev/null || { echo "$t is required"; exit 1; }
done
python3 -c "from PIL import features; assert features.check('libtiff')" 2>/dev/null || { echo "Pillow with libtiff is required"; exit 1; }
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b45_diff.py
