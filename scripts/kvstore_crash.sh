#!/usr/bin/env bash
# Does an acknowledged write actually survive the process dying?
#
# Every other check on the store runs its code. This one runs it and then
# kills it — SIGKILL, no cleanup, no handler, the way a power cut arrives —
# and asks whether every key the store *said* it had committed is still there
# when a new process opens the directory.
#
# That is the only question a store's durability claim is about, and it cannot
# be answered by a test that returns normally. Six kills at different moments,
# so the axe lands before a flush, during one, and after several.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
WORK="${TMPDIR:-/tmp}/grasm_kvcrash_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi
if ! "$GRASM" build apps/kvstore/src/main.gr --native -o "$WORK/kv" >/dev/null 2>&1; then
    echo "the store would not build"
    exit 1
fi

pass=0
fail=0
tables_seen=""

for t in 0.5 0.9 1.4 2.0 2.7 3.4; do
    d="$WORK/run-$t"
    mkdir -p "$d"
    "$WORK/kv" hammer "$d" 100000 >"$d/out.txt" 2>&1 &
    pid=$!
    ( sleep "$t"; kill -9 $pid 2>/dev/null ) &
    killer=$!
    wait $pid 2>/dev/null
    wait $killer 2>/dev/null

    # What survived, read off the disk. The process's own stdout cannot be
    # trusted here: that buffer dies with the process, so a store that
    # committed 400 and printed 200 would be judged on 200 — which is how the
    # first version of this gate failed, on itself rather than on the store.
    #
    # The property is that the survivors form a **prefix**. Keys are written
    # 0, 1, 2, ... in order, so a gap — key 500 present while 200 is gone — is
    # an acknowledged write that was lost.
    got=$("$WORK/kv" verify "$d" 100000 2>&1)
    run=$(sed -n 's/^prefix \([0-9]*\),.*/\1/p' <<<"$got")
    if [ -z "$run" ] || [ "$run" -lt 10 ]; then
        echo "  FAIL: killed at ${t}s and only $run keys survived — too few to prove anything"
        echo "        got: $got"
        fail=$((fail + 1))
        continue
    fi
    if [[ "$got" == *"wrong 0, beyond-the-gap 0"* ]]; then
        pass=$((pass + 1))
        tables_seen="$tables_seen $(sed 's/.*tables //' <<<"$got")"
    else
        echo "  FAIL: killed at ${t}s"
        echo "        got: $got"
        fail=$((fail + 1))
    fi
done

# A kill that never landed during a flush would leave the hardest case
# untested, so the spread of table counts is checked rather than assumed.
distinct=$(tr ' ' '\n' <<<"$tables_seen" | grep -c . || true)
levels=$(tr ' ' '\n' <<<"$tables_seen" | sort -u | grep -c . || true)
if [ "$levels" -lt 2 ]; then
    echo "  FAIL: every kill landed with the same number of tables ($tables_seen) —"
    echo "        the flush path was never interrupted, so this proved less than it looks"
    fail=$((fail + 1))
fi

echo "kills: $((pass + fail))   survived: $pass   lost data: $fail   table counts:$tables_seen"
[ "$fail" -eq 0 ]
