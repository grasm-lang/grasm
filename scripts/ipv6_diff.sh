#!/usr/bin/env bash
# IPv6 addresses against Python's own ipaddress.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/ipv6_diff.py
