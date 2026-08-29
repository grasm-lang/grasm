#!/usr/bin/env bash
# The measurement Phase 1 exists for: does the service survive *duration*?
#
# Nothing in this repository had ever run for more than a second or held more
# than a few thousand objects. Every gate is a program that starts, prints and
# exits — so the runtime's reference counting, which produced F79, had never
# been watched under sustained churn.
#
# This drives the service over a real socket for as long as it is given and
# samples its resident memory. What it is looking for is not throughput: it is
# whether the number *grows*.
#
#   apps/linkstore/load.sh [seconds] [port]
set -uo pipefail
cd "$(dirname "$0")/../.."

SECONDS_TO_RUN=${1:-60}
PORT=${2:-8080}
BIN=${BIN:-/tmp/linkstore.native}

[ -x "$BIN" ] || { echo "build it first: grasm build apps/linkstore/src/main.gr --native -o $BIN"; exit 1; }

"$BIN" -- serve >/tmp/linkstore-serve.log 2>&1 &
server=$!
trap 'kill $server 2>/dev/null' EXIT
sleep 1

if ! kill -0 $server 2>/dev/null; then
    echo "the server exited immediately:"; tail -5 /tmp/linkstore-serve.log; exit 1
fi

rss() { ps -o rss= -p "$1" 2>/dev/null | tr -d ' '; }

start_rss=$(rss $server)
echo "serving on $PORT, pid $server, rss ${start_rss}K"
echo
printf '%8s %10s %12s %10s\n' "elapsed" "requests" "rss(K)" "growth"

began=$(date +%s)
requests=0
sample_every=5
next_sample=$sample_every
created=""

while :; do
    now=$(date +%s)
    elapsed=$((now - began))
    [ "$elapsed" -ge "$SECONDS_TO_RUN" ] && break

    # A mix a real service sees: mostly reads, some writes, some redirects,
    # and some requests that are refused. A load test of one endpoint measures
    # one endpoint.
    curl -s -o /dev/null "http://127.0.0.1:$PORT/healthz"
    curl -s -o /dev/null "http://127.0.0.1:$PORT/"
    body=$(curl -s -X POST -d '{"target":"https://example.test/x"}' "http://127.0.0.1:$PORT/links")
    id=$(echo "$body" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
    [ -n "$id" ] && created="$id"
    [ -n "$created" ] && curl -s -o /dev/null "http://127.0.0.1:$PORT/r/$created"
    [ -n "$created" ] && curl -s -o /dev/null "http://127.0.0.1:$PORT/links/$created"
    curl -s -o /dev/null "http://127.0.0.1:$PORT/metrics"
    curl -s -o /dev/null -X POST -d 'not json' "http://127.0.0.1:$PORT/links"
    requests=$((requests + 7))

    if [ "$elapsed" -ge "$next_sample" ]; then
        r=$(rss $server)
        [ -z "$r" ] && { echo "the server died at ${elapsed}s after $requests requests"; tail -5 /tmp/linkstore-serve.log; exit 1; }
        printf '%7ss %10s %12s %+9s\n' "$elapsed" "$requests" "$r" "$((r - start_rss))"
        next_sample=$((next_sample + sample_every))
    fi
done

end_rss=$(rss $server)
echo
echo "$requests requests over ${SECONDS_TO_RUN}s"
echo "rss ${start_rss}K -> ${end_rss}K, growth $((end_rss - start_rss))K"
# The store grows on purpose — every POST adds a link — so some growth is the
# program working. What would be a defect is growth that does not stop when
# the store does, which is what a longer run distinguishes.
echo
echo "the store also grew on purpose: one link per POST, roughly $((requests / 7)) of them"
