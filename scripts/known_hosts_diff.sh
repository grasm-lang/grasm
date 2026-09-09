#!/usr/bin/env bash
# known_hosts matching against OpenSSH's own ssh-keygen.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/known_hosts_diff.py
