#!/usr/bin/env bash
# pcapng, 802.11, 802.15.4, Bluetooth LE and IKEv2 against tshark and scapy.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/packet4_diff.py
