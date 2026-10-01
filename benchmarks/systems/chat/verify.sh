#!/usr/bin/env bash
# Small correctness run: 50 connections in 5 channels of 10, 5 senders x 5
# messages. Every server must deliver exactly 5*5*9 = 225 messages.
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
CONNS="${CONNS:-50}" CHSIZE="${CHSIZE:-10}" MSGS="${MSGS:-5}" RATE="${RATE:-20}" \
  DRAIN="${DRAIN:-10s}" PORT="${PORT:-9200}" exec "$DIR/run.sh"
