#!/usr/bin/env bash
# Every gate, judged by exit code.
#
# A gate that fails says so in its exit status. Reading its summary line
# instead is how `wasmtest.sh` was reported green while it was exiting 1 over
# four stale committed modules -- the line said "identical: 184 mismatched: 0",
# which was true, and the failure was three lines further down. The same
# mistake in another shape hid nine of eleven `agreetest` failures behind a
# `tail -2`.
#
# So this reads one thing per gate: `$?`. The summary is printed for
# information and is never consulted. Full output goes to a log per gate, named
# on failure.
set -uo pipefail
cd "$(dirname "$0")/.."

LOGS=${LOGS:-$(mktemp -d)}
mkdir -p "$LOGS"
failed=0
declare -a FAILED=()

# Runs one gate and reports it. `$1` is the label, the rest is the command.
run_gate() {
    local name="$1"; shift
    local log="$LOGS/$name.log"
    printf '%-16s ' "$name"
    if "$@" >"$log" 2>&1; then
        # The last whole line carrying two or more counts, for information
        # only. Matching the counts themselves cut "not compiled: 0" down to
        # "compiled: 0", which says the opposite of what the gate found -- and
        # a runner built so a summary cannot mislead should not print a
        # misleading summary of its own.
        local note
        note=$(grep -hE '[a-z-]+: [0-9]+.*[a-z-]+: [0-9]+' "$log" | tail -1)
        echo "PASS  ${note:-ok}"
    else
        local code=$?
        echo "FAIL  (exit $code) $log"
        failed=$((failed + 1))
        FAILED+=("$name")
    fi
}

run_gate difftest      scripts/difftest.sh
run_gate leakcheck     scripts/leakcheck.sh
run_gate agreetest     scripts/agreetest.sh
run_gate nativegaps    scripts/nativegaps.sh
run_gate replaytest    scripts/replaytest.sh
run_gate wasmtest      scripts/wasmtest.sh
run_gate gatecheck     scripts/gatecheck.sh
run_gate crosstest     scripts/crosstest.sh
run_gate northstar     scripts/northstar.sh
run_gate gold          ProjectGold/run.sh
run_gate findings      scripts/findings_index.sh --check
run_gate clippy        cargo clippy --release --workspace --all-targets -- -D warnings
run_gate unittests     cargo test --release --workspace

for f in scripts/fuzz_*.py; do
    run_gate "$(basename "$f" .py)" python3 "$f"
done

echo
if [ "$failed" -gt 0 ]; then
    echo "$failed gate(s) failed: ${FAILED[*]}"
    echo "logs under $LOGS"
    exit 1
fi
echo "every gate passed"
