#!/usr/bin/env bash
# Does the broker hold up as a server, not just as a program?
#
# `difftest` runs every program with no arguments, `agreetest` compares the
# three ways of running one. Neither can see what a server actually has to get
# right: that a socket carries frames across a real kernel buffer, that a log
# survives the process being killed in the middle of a write, and that a
# follower ends up with the same bytes as the leader.
#
# So this starts real servers on real ports and checks four claims:
#
#   1. the three engines agree on `broker selftest`, which is deterministic
#   2. a published record comes back through fetch and through subscribe
#   3. SIGKILL during a write loses only unacknowledged records: the log
#      reopens, reports what it dropped, and the offsets continue
#   4. a follower converges on the leader's log, byte for byte
#
# Servers bind port 0 and print the port the kernel gave them, so two copies
# of this gate can run at once without agreeing on a number beforehand.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

SRC=apps/broker/src/main.gr
WORK="${TMPDIR:-/tmp}/grasm_broker_$$"
mkdir -p "$WORK"

PIDS=()
cleanup() {
    # Job control announces every killed background job on stderr, which is
    # noise in a gate that ends by killing four servers on purpose.
    exec 2>/dev/null
    for pid in ${PIDS+"${PIDS[@]}"}; do
        kill -9 "$pid" 2>/dev/null
        wait "$pid" 2>/dev/null
    done
    rm -rf "$WORK"
}
trap cleanup EXIT

pass=0
fail=0
declare -a FAILED=()

ok() {
    pass=$((pass + 1))
    echo "  ok    $1"
}

bad() {
    fail=$((fail + 1))
    FAILED+=("$1")
    echo "  FAIL  $1"
    if [ $# -gt 1 ]; then
        printf '        %s\n' "$2"
    fi
}

# Waits for a server to print the port it bound, and answers with it.
port_of() {
    local log="$1" tries=0
    while [ $tries -lt 200 ]; do
        local line
        line=$(head -1 "$log" 2>/dev/null)
        case "$line" in
            "broker listening on "*)
                echo "$line" | awk '{print $4}'
                return 0
                ;;
        esac
        sleep 0.05
        tries=$((tries + 1))
    done
    return 1
}

echo "broker: building"
if ! "$GRASM" build "$SRC" --native -o "$WORK/broker" >"$WORK/build.log" 2>&1; then
    echo "  BUILD FAILED (native)"
    sed -n '1,20p' "$WORK/build.log"
    exit 1
fi
if ! "$GRASM" build "$SRC" -o "$WORK/broker_bundled" >>"$WORK/build.log" 2>&1; then
    echo "  BUILD FAILED (bundled)"
    sed -n '1,20p' "$WORK/build.log"
    exit 1
fi
B="$WORK/broker"

# 1. The three engines on the deterministic self-test.
echo "broker: engines"
"$GRASM" run "$SRC" -- selftest --dir "$WORK/st_interp" >"$WORK/st_interp.out" 2>&1
interp_code=$?
"$B" selftest --dir "$WORK/st_native" >"$WORK/st_native.out" 2>&1
native_code=$?
"$WORK/broker_bundled" selftest --dir "$WORK/st_bundled" >"$WORK/st_bundled.out" 2>&1
bundled_code=$?

if [ "$interp_code" -eq 0 ]; then
    ok "the self-test holds under the interpreter"
else
    bad "the self-test holds under the interpreter" "$(tail -3 "$WORK/st_interp.out")"
fi
if diff -q "$WORK/st_interp.out" "$WORK/st_native.out" >/dev/null &&
    [ "$interp_code" -eq "$native_code" ]; then
    ok "the native build says the same thing"
else
    bad "the native build says the same thing" \
        "$(diff "$WORK/st_interp.out" "$WORK/st_native.out" | head -6)"
fi
if diff -q "$WORK/st_interp.out" "$WORK/st_bundled.out" >/dev/null &&
    [ "$interp_code" -eq "$bundled_code" ]; then
    ok "and so does the bundled one"
else
    bad "and so does the bundled one" \
        "$(diff "$WORK/st_interp.out" "$WORK/st_bundled.out" | head -6)"
fi

# The self-test writes files; running it twice must give the same answer, or
# it is testing the leftovers of the last run.
"$B" selftest --dir "$WORK/st_native" >"$WORK/st_again.out" 2>&1
if diff -q "$WORK/st_native.out" "$WORK/st_again.out" >/dev/null; then
    ok "and again over the same directory"
else
    bad "and again over the same directory" \
        "$(diff "$WORK/st_native.out" "$WORK/st_again.out" | head -6)"
fi

# 2. Over a socket: publish, fetch back, subscribe.
echo "broker: a live server"
mkdir -p "$WORK/live"
"$B" serve --port 0 --dir "$WORK/live" --topics bench >"$WORK/live.log" 2>&1 &
PIDS+=($!)
LIVE=$(port_of "$WORK/live.log")
if [ -z "${LIVE:-}" ]; then
    bad "the server binds a port" "$(cat "$WORK/live.log")"
else
    ok "the server binds a port"

    # One connection, so the order records are sent in is the order they are
    # written in and the payload can be checked against the offset.
    "$B" bench --port "$LIVE" --connections 1 --messages 2000 --size 96 \
        --depth 16 >"$WORK/bench.out" 2>&1
    if grep -q "2000 acknowledged" "$WORK/bench.out"; then
        ok "2000 records are published and acknowledged"
    else
        bad "2000 records are published and acknowledged" "$(cat "$WORK/bench.out")"
    fi

    "$B" verify --port "$LIVE" --topic bench --expect 2000 --size 96 \
        >"$WORK/verify.out" 2>&1
    if grep -q "read 2000 records, end offset 2000, 0 wrong" "$WORK/verify.out"; then
        ok "fetch reads every one of them back in order"
    else
        bad "fetch reads every one of them back in order" "$(cat "$WORK/verify.out")"
    fi

    "$B" tail --port "$LIVE" --topic bench --from 0 --expect 2000 --size 96 \
        >"$WORK/tail.out" 2>&1
    if grep -q "2000 records in .* 0 out of order or wrong" "$WORK/tail.out"; then
        ok "subscribe delivers them from the beginning"
    else
        bad "subscribe delivers them from the beginning" "$(cat "$WORK/tail.out")"
    fi

    "$B" stats --port "$LIVE" >"$WORK/stats.out" 2>&1
    if grep -q "topic bench first 0 end 2000" "$WORK/stats.out"; then
        ok "and the server's own count agrees"
    else
        bad "and the server's own count agrees" "$(cat "$WORK/stats.out")"
    fi
fi

# 3. Killed in the middle of a write.
echo "broker: killed mid-write"
mkdir -p "$WORK/crash"
"$B" serve --port 0 --dir "$WORK/crash" --topics bench >"$WORK/crash1.log" 2>&1 &
victim=$!
PIDS+=($victim)
CRASH=$(port_of "$WORK/crash1.log")
if [ -z "${CRASH:-}" ]; then
    bad "the server to kill starts" "$(cat "$WORK/crash1.log")"
else
    "$B" bench --port "$CRASH" --connections 4 --messages 400000 --size 128 \
        --depth 32 >"$WORK/crash_bench.out" 2>&1 &
    writer=$!
    PIDS+=($writer)
    sleep 1
    kill -9 "$victim" 2>/dev/null
    kill -9 "$writer" 2>/dev/null
    { wait "$victim"; wait "$writer"; } 2>/dev/null

    # Reopen the same directory. Nothing was flushed on the way out, so
    # whatever is there is what the kernel had already taken.
    "$B" serve --port 0 --dir "$WORK/crash" --topics bench >"$WORK/crash2.log" 2>&1 &
    PIDS+=($!)
    AGAIN=$(port_of "$WORK/crash2.log")
    if [ -z "${AGAIN:-}" ]; then
        bad "the log reopens after a kill" "$(cat "$WORK/crash2.log")"
    else
        ok "the log reopens after a kill"
        "$B" stats --port "$AGAIN" >"$WORK/crash_stats.out" 2>&1
        kept=$(awk '/^topic bench/ {print $6}' "$WORK/crash_stats.out")
        if [ -n "${kept:-}" ] && [ "$kept" -gt 100 ]; then
            ok "it kept the $kept records that had landed"
        else
            bad "it kept the records that had landed" "$(cat "$WORK/crash_stats.out")"
            kept=0
        fi

        # Every surviving record has to be whole: a half-written tail is
        # dropped at startup, never handed to a reader.
        "$B" verify --port "$AGAIN" --topic bench --expect "$kept" --size 128 \
            >"$WORK/crash_verify.out" 2>&1
        if grep -q "read $kept records, end offset $kept, 0 wrong" \
            "$WORK/crash_verify.out"; then
            ok "and every one of them reads back whole"
        else
            bad "and every one of them reads back whole" \
                "$(cat "$WORK/crash_verify.out")"
        fi

        # And the log carries on from where it stopped rather than from zero.
        "$B" bench --port "$AGAIN" --connections 1 --messages 500 --size 128 \
            --depth 8 >/dev/null 2>&1
        "$B" stats --port "$AGAIN" >"$WORK/crash_stats2.out" 2>&1
        want=$((kept + 500))
        now=$(awk '/^topic bench/ {print $6}' "$WORK/crash_stats2.out")
        if [ "${now:-0}" -eq "$want" ]; then
            ok "and the offsets continue at $kept rather than at zero"
        else
            bad "and the offsets continue rather than start again" \
                "expected end $want, got ${now:-none}"
        fi
    fi
fi

# 4. A follower is a subscriber that writes what it hears.
echo "broker: replication"
mkdir -p "$WORK/lead" "$WORK/foll"
"$B" serve --port 0 --dir "$WORK/lead" --topics bench >"$WORK/lead.log" 2>&1 &
PIDS+=($!)
LEAD=$(port_of "$WORK/lead.log")
if [ -z "${LEAD:-}" ]; then
    bad "the leader starts" "$(cat "$WORK/lead.log")"
else
    "$B" serve --port 0 --dir "$WORK/foll" --peer "127.0.0.1:$LEAD" \
        --topics bench >"$WORK/foll.log" 2>&1 &
    PIDS+=($!)
    FOLL=$(port_of "$WORK/foll.log")
    if [ -z "${FOLL:-}" ]; then
        bad "the follower starts" "$(cat "$WORK/foll.log")"
    else
        ok "the follower attaches to the leader"
        "$B" bench --port "$LEAD" --connections 1 --messages 3000 --size 64 \
            --depth 16 >/dev/null 2>&1

        # Replication is asynchronous, so it is given a moment — but a
        # bounded one, or a broken follower would hang the gate.
        waited=0
        end=0
        while [ $waited -lt 100 ]; do
            "$B" stats --port "$FOLL" >"$WORK/foll_stats.out" 2>&1
            end=$(awk '/^topic bench/ {print $6}' "$WORK/foll_stats.out")
            if [ "${end:-0}" -eq 3000 ]; then
                break
            fi
            sleep 0.1
            waited=$((waited + 1))
        done
        if [ "${end:-0}" -eq 3000 ]; then
            ok "it catches up to all 3000 records"
        else
            bad "it catches up to all 3000 records" \
                "follower stopped at ${end:-none}"
        fi

        "$B" verify --port "$FOLL" --topic bench --expect 3000 --size 64 \
            >"$WORK/foll_verify.out" 2>&1
        if grep -q "read 3000 records, end offset 3000, 0 wrong" \
            "$WORK/foll_verify.out"; then
            ok "and serves them itself, in the leader's order"
        else
            bad "and serves them itself, in the leader's order" \
                "$(cat "$WORK/foll_verify.out")"
        fi

        # The point of replication is the same bytes, so compare the files.
        if diff -r "$WORK/lead/bench" "$WORK/foll/bench" >"$WORK/rep_diff.out" 2>&1; then
            ok "the follower's segments match the leader's byte for byte"
        else
            bad "the follower's segments match the leader's byte for byte" \
                "$(head -4 "$WORK/rep_diff.out")"
        fi
    fi
fi

echo
echo "broker_test: $pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then
    for name in "${FAILED[@]}"; do
        echo "  - $name"
    done
    exit 1
fi
