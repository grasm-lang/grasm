#!/usr/bin/env bash
# Chat broadcast benchmark: build the three servers and the load generator,
# then, for each server: start it, drive it with the loadgen, stop it.
#
# Knobs (environment):
#   CONNS=10000   client connections (see README: ~16k is the ceiling to one
#                 127.0.0.1 port on macOS)
#   CHSIZE=1000   clients per channel  (channels = CONNS / CHSIZE)
#   SENDERS=      clients that send (default CONNS/10)
#   MSGS=10       messages per sender
#   RATE=1        messages per second per sender
#   PORT=9100     first port; the servers use PORT, PORT+1, PORT+2
#   SERVERS="grasm rust go"
#   DRAIN=60s     give up this long after deliveries stop arriving
#   LADDRS=       loadgen source IPs, comma separated (to go past ~16k
#                 connections; see README "Going higher")
#
# Exits non-zero if any server's delivery count is not exactly the expected one.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../../.." && pwd)"
OUT="${OUT:-${TMPDIR:-/tmp}/grasm-systems-chat}"
CONNS="${CONNS:-10000}"
CHSIZE="${CHSIZE:-1000}"
SENDERS="${SENDERS:--1}"
MSGS="${MSGS:-10}"
RATE="${RATE:-1}"
PORT="${PORT:-9100}"
SERVERS="${SERVERS:-grasm rust go}"
DRAIN="${DRAIN:-60s}"
LADDRS="${LADDRS:-}"
GRASM="${GRASM:-$ROOT/target/release/grasm}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$OUT/rust-target}"

mkdir -p "$OUT"
echo "== build" >&2
"$GRASM" build "$DIR/grasm/server.gr" --native -o "$OUT/grasm-server" >&2
(cd "$DIR/rust" && cargo build --release --offline -q)
cp "$CARGO_TARGET_DIR/release/chat-server" "$OUT/rust-server"
(cd "$DIR/go" && go build -o "$OUT/go-server" .)
(cd "$DIR/loadgen" && go build -o "$OUT/loadgen" .)

# A server and a loadgen at 10k connections each need well over 10k fds.
ulimit -n 65536 2>/dev/null || ulimit -n "$(ulimit -Hn)" 2>/dev/null || true

SERVER_PID=""
cleanup() { [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

status=0
results=()
port=$PORT
for name in $SERVERS; do
  log="$OUT/$name-server.log"
  "$OUT/$name-server" "$port" >"$log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 200); do
    grep -q '^listening' "$log" 2>/dev/null && break
    sleep 0.05
  done
  if ! grep -q '^listening' "$log"; then
    echo "$name: server did not start:" >&2; cat "$log" >&2; status=1; continue
  fi
  echo "== $name (pid $SERVER_PID, port $port)" >&2
  line=$("$OUT/loadgen" -name "$name" -addrs "127.0.0.1:$port" -conns "$CONNS" \
        -chsize "$CHSIZE" -senders "$SENDERS" -msgs "$MSGS" -rate "$RATE" \
        -pid "$SERVER_PID" -drain "$DRAIN" -laddrs "$LADDRS" | grep '^RESULT') || status=1
  kill "$SERVER_PID" 2>/dev/null || true
  wait "$SERVER_PID" 2>/dev/null || true
  SERVER_PID=""
  [ -n "$line" ] && results+=("$line") && echo "$line" >&2
  case "$line" in *"exact=true"*) ;; *) echo "$name: delivery count NOT exact" >&2; status=1 ;; esac
  port=$((port + 1))
  line=""
done

field() { sed -n "s/.* $1=\([^ ]*\).*/\1/p" <<<"$2"; }
echo
printf '%-6s %12s %12s %8s %14s %9s %9s %9s %12s\n' server deliveries expected exact "deliveries/s" p50_ms p99_ms p999_ms peak_rss_MB
for r in ${results[@]+"${results[@]}"}; do
  printf '%-6s %12s %12s %8s %14s %9s %9s %9s %12s\n' \
    "$(field name "$r")" "$(field deliveries "$r")" "$(field expected "$r")" "$(field exact "$r")" \
    "$(field throughput "$r")" "$(field p50_ms "$r")" "$(field p99_ms "$r")" "$(field p999_ms "$r")" "$(field peak_rss_mb "$r")"
done
exit $status
