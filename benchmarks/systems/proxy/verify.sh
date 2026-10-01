#!/usr/bin/env bash
# Small correctness run: 4 connections, 1,000 requests through each proxy.
# Every response is checked by the load generator (status, body length,
# X-Forwarded-For seen by the upstream, request body length); exits non-zero
# unless all three proxies have zero errors.
set -uo pipefail
cd "$(dirname "$0")"
[ "${SKIP_BUILD:-0}" = 1 ] || ./build.sh || exit 1
source ./common.sh

start_upstream || exit 1
status=0
for name in ${PROXIES:-grasm rust go}; do
    start_proxy "$name" || { status=1; continue; }
    if "$BUILD_DIR/loadgen" -addr "127.0.0.1:$PROXY_PORT" -c 4 -n 1000 -size "$SIZE" \
        -pid "$PROXY_PID" -name "$name" -v; then
        echo "PASS $name"
    else
        echo "FAIL $name"
        status=1
    fi
    stop_proxy
done
exit $status
