#!/usr/bin/env bash
# Image formats against Pillow: QOI, PNM, TGA, PCX, XBM, XPM, SGI, DDS,
# ICNS, Sun raster and APNG.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b37_diff.py
