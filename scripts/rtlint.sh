#!/usr/bin/env bash
# The runtime's lint ([T-RUNTIME], docs/self-hosting.md §5.6): no plain
# load or store of a process cell outside a function marked as holding its
# lock (scripts/rtlint.py says what it follows and what it refuses).
#
#   * compiler/rt and the dialect's fixture must lint clean;
#   * the lint's own cases (scripts/fixtures/rtlint, one module each) must
#     each get exactly the refusals they say they expect;
#   * two controls: a plain store planted in the fixture must be refused,
#     and the lint planted to take a plain store for an atomic one must
#     fail its cases.
#
# Every count is derived and asserted: a lint that read no cell checked
# nothing.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
export GRASM
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

failed=0
if ! python3 scripts/rtlint.py compiler/rt/src/main.gr scripts/fixtures/rtdialect/src/main.gr >"$WORK/lint.out" 2>&1; then
    echo "the runtime does not lint clean:"
    sed 's/^/  /' "$WORK/lint.out" | head -20
    failed=1
fi
tail -1 "$WORK/lint.out"
cells=$(sed -n 's/.*process cells: \([0-9]*\).*/\1/p' "$WORK/lint.out")
if [ "${cells:-0}" -lt 6 ]; then
    echo "only ${cells:-0} process cells were read; the fixture alone has 6"
    failed=1
fi

if ! python3 scripts/rtlint.py --cases scripts/fixtures/rtlint/src/main.gr >"$WORK/cases.out" 2>&1; then
    sed 's/^/  /' "$WORK/cases.out" | head -20
    failed=1
fi
tail -1 "$WORK/cases.out"
ncases=$(sed -n 's/^cases: \([0-9]*\) .*/\1/p' "$WORK/cases.out")
nagreed=$(sed -n 's/.*as expected: \([0-9]*\).*/\1/p' "$WORK/cases.out")
if [ "${ncases:-0}" -lt 19 ] || [ "${nagreed:-0}" != "${ncases:-0}" ]; then
    echo "cases: ${ncases:-0}, as expected: ${nagreed:-0}; every one of at least 19 must be"
    failed=1
fi

# Controls.
caught=0
cp -R scripts/fixtures/rtdialect "$WORK/plain"
python3 - "$WORK/plain/src/main.gr" <<'EOF'
import sys
p = sys.argv[1]
s = open(p).read()
old = "        mem_process(HITS).atomic_add_i64(1, SeqCst) + 1"
new = "        let c = mem_process(HITS)\n        c.store_i64(c.atomic_load_i64(SeqCst) + 1)\n        c.atomic_load_i64(SeqCst)"
assert s.count(old) == 1
open(p, "w").write(s.replace(old, new))
EOF
if python3 scripts/rtlint.py "$WORK/plain/src/main.gr" >"$WORK/c1.out" 2>&1; then
    echo "MISSED: plain-store -- a plain store to a shared cell passed"
elif grep -q 'plain: a plain store of process cell `main::HITS`' "$WORK/c1.out"; then
    caught=$((caught + 1))
    echo "caught: plain-store -- $(grep -h 'plain store' "$WORK/c1.out" | head -1 | cut -c1-150)"
else
    echo "MISSED: plain-store failed for another reason: $(tail -1 "$WORK/c1.out")"
fi
if RTLINT_FAULT=store-atomic python3 scripts/rtlint.py --cases scripts/fixtures/rtlint/src/main.gr >"$WORK/c2.out" 2>&1; then
    echo "MISSED: lint-blind -- the cases passed with stores taken for atomic"
elif grep -q '^CASE .*bad_store' "$WORK/c2.out"; then
    caught=$((caught + 1))
    echo "caught: lint-blind -- $(grep -h '^CASE' "$WORK/c2.out" | head -1 | cut -c1-150)"
else
    echo "MISSED: lint-blind failed for another reason: $(tail -1 "$WORK/c2.out")"
fi
echo "controls: 2   caught: $caught   missed: $((2 - caught))"
[ "$caught" = 2 ] || failed=1
exit $failed
