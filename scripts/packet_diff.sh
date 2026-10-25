#!/usr/bin/env bash
# Packet headers against scapy.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/packet_diff.py
