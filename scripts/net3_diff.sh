#!/usr/bin/env bash
# inet_aton, netdb, mail dates, Maildir and ftplib, against Python.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/net3_diff.py
