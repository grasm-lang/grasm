#!/usr/bin/env bash
# [E-RECURSION-DEPTH] guarantees at least 10,000 nested calls. Nothing
# measured how much margin the interpreter actually has over that, and the
# margin moved: ProjectGold/08-fibonacci asserted depth(50000) and passed on
# 2026-09-13, then panicked on 2026-10-14 with the same stack budget. A
# frame had grown. The only thing depending on the margin was one test's
# expectation, so when it shrank the failure read as an engine disagreement
# (F99) rather than as what it was.
#
# This gate measures the margin instead of assuming it. Two checks:
#
#   1. The contract. 10,000 nested calls must work on every engine, and must
#      work through mutual recursion too, which counts against the same bound.
#   2. The margin. The interpreter's real capacity is bisected and compared
#      against a floor of twice the guarantee. Past its own bound a program
#      must *panic* and not abort, so exceeding capacity is not a failure --
#      losing the margin is.
#
# The bytes-per-frame figure is printed because it is the thing that actually
# moves: capacity is STACK_BUDGET divided by it, and the budget is a constant.
#
# Capacity is a property of a *shape*, not of the language: this gate's `down`
# measures ~31,000 where 08-fibonacci's `depth` measured ~42,000 on the same
# build, because a frame holds that function's locals. So the figure is only
# comparable against itself, which is why the shape below is fixed and should
# not be tidied -- changing it moves the number without anything changing.
set -u
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The guaranteed floor, and the margin this gate insists on keeping over it.
# Twice is not a spec number -- it is the smallest margin in which a frame
# doubling is caught by this gate instead of by the guarantee failing.
GUARANTEED=10000
FLOOR=$((GUARANTEED * 2))
# STACK_BUDGET in crates/grasm_interp/src/interp.rs, release profile.
BUDGET_MB=448

prog() {  # $1 = depth
  cat <<GR
fn down(n: i64) -> i64 {
    if n <= 0 { 0 } else { down(n - 1) + 1 }
}

fn main() {
    print("{down($1)}")
}
GR
}

# Exit 0 if the interpreter reaches $1 and prints it.
reaches() {
  prog "$1" > "$WORK/d.gr"
  [ "$("$GRASM" run "$WORK/d.gr" 2>/dev/null)" = "$1" ]
}

fail=0

# 1. The contract, on all three engines, straight and mutual.
echo "the guarantee: $GUARANTEED nested calls on every engine"
prog "$GUARANTEED" > "$WORK/g.gr"
want=$GUARANTEED
got=$("$GRASM" run "$WORK/g.gr" 2>/dev/null)
[ "$got" = "$want" ] && echo "    interpreter: $got" || { echo "    interpreter: FAILED ($got)"; fail=1; }

if "$GRASM" build "$WORK/g.gr" -o "$WORK/g.bundle" >/dev/null 2>&1; then
    got=$("$WORK/g.bundle" 2>/dev/null)
    [ "$got" = "$want" ] && echo "    bundle:      $got" || { echo "    bundle:      FAILED ($got)"; fail=1; }
else
    echo "    bundle:      did not build"; fail=1
fi

if "$GRASM" build "$WORK/g.gr" --native -o "$WORK/g.native" >/dev/null 2>&1; then
    got=$("$WORK/g.native" 2>/dev/null)
    [ "$got" = "$want" ] && echo "    native:      $got" || { echo "    native:      FAILED ($got)"; fail=1; }
else
    echo "    native:      did not build"; fail=1
fi

# Mutual recursion counts against the same bound, so it is part of the
# contract and not a separate curiosity.
cat > "$WORK/m.gr" <<GR
fn ping(n: i64) -> i64 { if n <= 0 { 0 } else { pong(n - 1) + 1 } }
fn pong(n: i64) -> i64 { if n <= 0 { 0 } else { ping(n - 1) + 1 } }

fn main() {
    print("{ping($GUARANTEED)}")
}
GR
got=$("$GRASM" run "$WORK/m.gr" 2>/dev/null)
[ "$got" = "$want" ] && echo "    mutual:      $got" || { echo "    mutual:      FAILED ($got)"; fail=1; }

# 2. The margin. Bisect the interpreter's real capacity between the guarantee
#    and MAX_DEPTH, which is the depth guard's own ceiling.
echo "the margin: bisecting the interpreter's capacity"
lo=$GUARANTEED
hi=100000
if reaches "$hi"; then
    echo "    reaches MAX_DEPTH -- the depth guard stops it, not the stack"
    echo "    capacity: $hi nested calls (the guard's own ceiling)"
    cap=$hi
else
    while [ $((hi - lo)) -gt 500 ]; do
        mid=$(( (lo + hi) / 2 ))
        if reaches "$mid"; then lo=$mid; else hi=$mid; fi
    done
    cap=$lo
fi
bytes=$(( BUDGET_MB * 1024 * 1024 / cap ))
echo "    capacity: $cap nested calls (bisected to within 500)"
echo "    per frame: ~$bytes bytes of the ${BUDGET_MB} MB budget"
echo "    margin:    $(( cap / GUARANTEED ))x the guarantee (floor is $(( FLOOR / GUARANTEED ))x)"

# Past its own bound the program must panic, not abort: a signal here would
# be the defect [E-RECURSION-DEPTH] names, and the engines are licensed to
# stop in different places only because this holds.
over=$(( cap * 2 ))
prog "$over" > "$WORK/o.gr"
out=$("$GRASM" run "$WORK/o.gr" 2>&1); code=$?
case "$code" in
  101) echo "    past capacity ($over): panics, exit 101" ;;
  0)   echo "    past capacity ($over): completed -- capacity measurement is wrong"; fail=1 ;;
  *)   echo "    past capacity ($over): exit $code, not a panic -- aborted where it must panic"
       echo "        $(printf '%s' "$out" | tail -1 | cut -c1-80)"; fail=1 ;;
esac

if [ "$cap" -lt "$FLOOR" ]; then
    echo
    echo "capacity $cap is under the floor of $FLOOR."
    echo "The guarantee is $GUARANTEED and nothing here raised the stack budget,"
    echo "so an interpreter frame got fatter -- it is ~$bytes bytes now. Either"
    echo "shrink the frame or raise STACK_BUDGET in crates/grasm_interp/src/"
    echo "interp.rs, and do not lower this floor to make the gate pass: the"
    echo "floor exists so a frame doubling is caught here rather than by"
    echo "[E-RECURSION-DEPTH] failing."
    fail=1
fi

[ "$fail" -eq 0 ] && echo "depth: contract holds, margin $(( cap / GUARANTEED ))x" || echo "depth: FAILED"
exit "$fail"
