#!/usr/bin/env bash
# HTML charset sniffing, SVCB, DNSSEC and WKB against x/net, miekg/dns and orb.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/b28b_diff.py
