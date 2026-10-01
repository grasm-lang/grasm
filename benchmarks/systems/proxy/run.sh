#!/usr/bin/env bash
# Full benchmark: for each proxy, start it, drive it with the load generator at
# full size, stop it. Environment: CONNS (256), REQS (200000), SIZE (4096),
# PROXIES ("grasm rust go"), PORT_BASE (23300), BUILD_DIR, SKIP_BUILD=1.
set -uo pipefail
cd "$(dirname "$0")"
CONNS="${CONNS:-256}"
REQS="${REQS:-200000}"
PROXIES="${PROXIES:-grasm rust go}"
[ "${SKIP_BUILD:-0}" = 1 ] || ./build.sh || exit 1
source ./common.sh

start_upstream || exit 1
echo "proxy bench: $CONNS connections, $REQS requests, ${SIZE}-byte bodies"
printf '%-6s %10s %9s %9s %9s %12s %7s\n' name req/s p50_ms p99_ms p999_ms peak_rss_MB errors
status=0
for name in $PROXIES; do
    start_proxy "$name" || { status=1; continue; }
    line=$("$BUILD_DIR/loadgen" -addr "127.0.0.1:$PROXY_PORT" -c "$CONNS" -n "$REQS" \
        -size "$SIZE" -pid "$PROXY_PID" -name "$name" -v) || status=1
    stop_proxy
    get() { sed -n "s/.* $1=\([^ ]*\).*/\1/p" <<<"$line" | sed 's/ms$//; s/MB$//'; }
    printf '%-6s %10s %9s %9s %9s %12s %7s\n' "$name" "$(get rps)" "$(get p50)" \
        "$(get p99)" "$(get p999)" "$(get peak_rss)" "$(get errors)"
done
exit $status
