#!/usr/bin/env bash
# RPM version ordering against rpm's own rpmvercmp suite.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/rpm_vercmp_diff.py
