# Shared by run.sh and verify.sh: start/stop helpers.
HERE="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="${BUILD_DIR:-${TMPDIR:-/tmp}/proxybench-build}"
PORT_BASE="${PORT_BASE:-23300}"
UP_PORT=$PORT_BASE
PROXY_PORT=$((PORT_BASE + 1))
SIZE="${SIZE:-4096}"
PIDS=()

cleanup() {
    for p in "${PIDS[@]:-}"; do
        [ -n "$p" ] && kill "$p" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# wait_port PORT: up to ~10 s for something to accept on it. A freshly built
# binary's first exec on macOS can take ~0.5 s, so starting without waiting
# loses the first connections.
wait_port() {
    for _ in $(seq 100); do
        nc -z 127.0.0.1 "$1" 2>/dev/null && return 0
        perl -e 'select(undef,undef,undef,0.1)'
    done
    echo "nothing listening on $1" >&2
    return 1
}

start_upstream() {
    "$BUILD_DIR/upstream" -port "$UP_PORT" -size "$SIZE" &
    UPSTREAM_PID=$!
    PIDS+=("$UPSTREAM_PID")
    wait_port "$UP_PORT"
}

# start_proxy NAME -> sets PROXY_PID
start_proxy() {
    "$BUILD_DIR/proxy-$1" "$PROXY_PORT" "$UP_PORT" >/dev/null &
    PROXY_PID=$!
    PIDS+=("$PROXY_PID")
    wait_port "$PROXY_PORT"
}

stop_proxy() {
    kill "$PROXY_PID" 2>/dev/null || true
    wait "$PROXY_PID" 2>/dev/null || true
}
