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
run_gate fmttest       scripts/fmttest.sh
run_gate linttest      scripts/linttest.sh
run_gate grasmwire     scripts/grasmwire_e2e.sh
run_gate cryptodiff    scripts/crypto_diff.sh
run_gate certdiff      scripts/cert_diff.sh
run_gate kvcrash       scripts/kvstore_crash.sh
run_gate zlibdiff      scripts/zlib_diff.sh
run_gate casefolddiff  scripts/casefold_diff.sh
run_gate wasmdiff      scripts/wasm_diff.sh
run_gate asminterop    scripts/asm_interop.sh
run_gate gitinterop    scripts/git_interop.sh
run_gate northstar     scripts/northstar.sh
run_gate gold          ProjectGold/run.sh
run_gate findings      scripts/findings_index.sh --check
run_gate clippy        cargo clippy --release --workspace --all-targets -- -D warnings
run_gate unittests     cargo test --release --workspace

for f in scripts/fuzz_*.py; do
    run_gate "$(basename "$f" .py)" python3 "$f"
done

# The site states what the gates found. Rather than let somebody retype a
# figure, the run that proved it writes it down and the site is regenerated
# from that record, so a number on the page is the last green run's or the
# page does not build. Last, because there is nothing to record until the
# gates above have finished.
site_facts() {
    python3 - "$LOGS" <<'FACTS'
import json, os, re, sys

logs = sys.argv[1]
# gate log -> (the site's name for it, the count that gate's summary prints)
WANT = {
    "difftest":  [("corpus", "identical")],
    "wasmtest":  [("wasm_ok", "identical"), ("wasm_skip", "skipped")],
    "crosstest": [("cross", "identical")],
    "gold":      [("agreeing", "agreeing")],
    "fuzz_laws": [("laws", "laws")],
}
out = {}
for gate, wants in sorted(WANT.items()):
    path = os.path.join(logs, gate + ".log")
    if not os.path.exists(path):
        print(f"the {gate} gate left no log, so its figure cannot be recorded")
        sys.exit(1)
    text = open(path).read()
    for key, field in wants:
        hit = re.findall(r"\b" + field + r": ([0-9]+)", text)
        if not hit:
            print(f"the {gate} gate no longer prints '{field}:'. Writing the")
            print("site from this run would state a figure nobody measured.")
            sys.exit(1)
        out[key] = int(hit[-1])
os.makedirs("site", exist_ok=True)
with open("site/facts.json", "w") as f:
    json.dump(out, f, indent=2, sort_keys=True)
    f.write("\n")
print("   ".join(f"{k}: {v}" for k, v in sorted(out.items())))
FACTS
}

if [ "$failed" -eq 0 ]; then
    run_gate sitefacts site_facts
    run_gate site      python3 scripts/site.py
fi

echo
if [ "$failed" -gt 0 ]; then
    echo "$failed gate(s) failed: ${FAILED[*]}"
    echo "logs under $LOGS"
    exit 1
fi
echo "every gate passed"
