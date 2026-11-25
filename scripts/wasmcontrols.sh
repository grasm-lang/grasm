#!/usr/bin/env bash
# Mutation controls for the wasm32 target's gates (docs/self-hosting.md §6.24,
# S8 part seven): a gate that cannot fail checks nothing.
#
# Each control copies `compiler/` aside, plants one fault (`scripts/
# s3_mutate.py`, which refuses a mutation that matches nothing), builds that
# driver, and runs the gate named for it with `S3=` and `WASM_LIBC=` pointing
# into the copy, on the `examples/` programs for `wasmsubset` (a planted fault
# needs a few programs to show, not the corpus) and stopping at the first
# failure. The gate must fail, and for the planted reason: its log must show
# the symptom that fault produces.
#
#   w-add        `add` selected as `i64.sub` -> wasmsubset (not as grasm run)
#   w-label      a branch's label one level out -> wasmsubset (the module is
#                invalid: wasm_validate.py refuses it, as V8 does)
#   w-local-type the `i64` locals declared `f64` -> wasmsubset
#                (wasm_validate.py refuses the module)
#   w-switch     the context switch's leaf returns rather than refusing, so a
#                `concurrent` block runs on -> wasmsubset (the refusals)
#   w-words      the C library words a refusal otherwise than the Rust
#                module -> wasmsubset (the refusals)
#   w-nondet     the driver writes the clock into every module -> wasmsubset
#                (built twice, not the same bytes)
#   w-floor      no shadow frame for a recursion, so the engine's stack
#                overflows before the floor -> wasmsubset (the stack)
#
# And the hop to fresh engine stacks (§6.26, S8 part nine), each by
# `wasmsubset`'s stack:
#
#   w-hop        the segment's check never hops, so a recursion runs on one
#                engine stack until it overflows -> past any depth
#   w-small      frames sized too small (the scale 0.80, below every shape's
#                smallest safe one), so the engine's stack overflows before
#                the segment's floor -> past any depth
#   w-tier       the scale 1.00: above every shape's smallest safe one under
#                V8's default tiering (0.95 at most) and below `mutual`'s with
#                TurboFan from the start (1.02), so only the tiers show it
#                -> under --no-liftoff, past any depth
#   w-deep-catch `_deep` does not catch an `exit` or a panic, so the panic
#                past the bound, thrown on a deeper stack, is lost there and
#                the run traps -> past any depth
#   w-done       the hop's completion check dropped, so a stack that trapped
#                is taken for one that answered -> a trap in a deeper stack
#                not ended as a trap
#   w-seg-nondet the driver takes the segment's size from the clock, so two
#                builds differ -> not the same bytes
#   w-deep-words without JSPI the hop refuses in other words than the
#                clause's -> without JSPI, past any depth
#   w-leb        a signed LEB128 ends a byte early when bit 5 is clear rather
#                than bit 6 -> encodetest_wasm
#   w-libm       `sin`'s kernel's first coefficient one bit off
#                -> fuzz_wasmmath
#
# A section written after a gate's name (`gate:section`) runs that section
# alone: `PLAYTEST_ONLY` for `playtest`, `WASM_SUBSET_ONLY` for `wasmsubset`.
#
# And the playground's (§6.25, S8 part eight), each by `playtest` on the
# sections that show it:
#
#   w-nondet     (above) -> playtest (built twice, not the same bytes)
#   p-frame      the answer's status and output length change places
#                -> playtest frame (not as the Rust module)
#   p-words      "cannot replay" worded otherwise -> playtest frame
#   p-journal    a journal handed over and not replayed -> playtest replay
#   p-reset      a run's reset leaves the cells as the last run left them
#                -> playtest session (state carried)
#   p-stderr     standard error dropped -> playtest frame
#   p-hop        the switch refused under JSPI too -> playtest stack
#   p-nostack    without JSPI, the interpreter hops by room all the same
#                -> playtest stack (not the panic)
#   p-dirty      a large mapping reused without zeroing -> playtest stack
#                (mapzero)
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

controls=0
caught=0
declare -a MISSED=()

# control NAME FILE OLD NEW -- GATE SYMPTOM [GATE SYMPTOM] ...
control() {
    local name=$1 file=$2 old=$3 new=$4
    shift 5
    rm -rf "$WORK/src"
    mkdir -p "$WORK/src"
    cp -R compiler "$WORK/src/"
    # The playground's sources and what they import, for `playtest`.
    mkdir -p "$WORK/src/tools" "$WORK/src/packages"
    cp -R tools/playground tools/tirinterp tools/gfmt "$WORK/src/tools/"
    cp -R packages/toml "$WORK/src/packages/"
    if ! python3 -B scripts/s3_mutate.py "$WORK/src/$file" "$old" "$new"; then
        MISSED+=("$name: the mutation does not apply")
        return
    fi
    local driver="$WORK/s3-$name"
    if ! "$GRASM" build "$WORK/src/compiler/s3/src/main.gr" --native -o "$driver" >"$WORK/build-$name.log" 2>&1; then
        MISSED+=("$name: the mutated driver does not build")
        sed 's/^/    /' "$WORK/build-$name.log" | head -5
        return
    fi
    while [ $# -ge 2 ]; do
        local gate=${1%%:*} only="" symptom=$2
        [ "$gate" != "$1" ] && only=${1#*:}
        shift 2
        controls=$((controls + 1))
        local log="$WORK/$name-$(basename "$gate" .sh).log"
        S3="$driver" WASM_LIBC="$WORK/src/compiler/rtwasm" PLAYGROUND="$WORK/src" DIRS=examples \
            WASM_SUBSET_STOP=1 PLAYTEST_STOP=1 PLAYTEST_ONLY="$only" WASM_SUBSET_ONLY="$only" \
            FUZZ_WASMMATH_SEEDS=1 FUZZ_WASMMATH_CALLS=4000 "$gate" >"$log" 2>&1
        local code=$?
        if [ "$code" != 0 ] && grep -cF -- "$symptom" "$log" >/dev/null; then
            caught=$((caught + 1))
            echo "caught: $name by $(basename "$gate" .sh) (exit $code; \"$symptom\")"
        elif [ "$code" != 0 ]; then
            MISSED+=("$name: $(basename "$gate" .sh) failed (exit $code) but not with \"$symptom\"")
            tail -5 "$log" | sed 's/^/    /'
        else
            MISSED+=("$name: $(basename "$gate" .sh) passed with the fault in place")
        fi
    done
}

B=compiler/wasm32/src/body.gr

control w-add $B \
    "    } else if op == ir.M_ADD {
        binop(b, s, f, i, enc.OP_I64_ADD)" \
    "    } else if op == ir.M_ADD {
        binop(b, s, f, i, enc.OP_I64_SUB)" -- \
    scripts/wasmsubset.sh "not as grasm run"

control w-label $B \
    "            return ctx.len() - 1 - k" "            return ctx.len() - k" -- \
    scripts/wasmsubset.sh "INVALID"

control w-local-type $B \
    "        enc.uleb(b, ni)
        enc.byte(b, enc.T_I64)" \
    "        enc.uleb(b, ni)
        enc.byte(b, enc.T_F64)" -- \
    scripts/wasmsubset.sh "INVALID"

control w-switch compiler/rtwasm/src/sys.gr \
    'export fn wasm_no_switch() {
    unsafe {' \
    'export fn wasm_no_switch() {
    return ()
    unsafe {' -- \
    scripts/wasmsubset.sh "not refused as the Rust module refuses it"

control w-words compiler/rtwasm/src/sys.gr \
    '    unsafe { mem_static("operation not supported on this platform") }' \
    '    unsafe { mem_static("operation not supported") }' -- \
    scripts/wasmsubset.sh "not refused as the Rust module refuses it"

control w-nondet compiler/s3/src/main.gr \
    "    match write_bytes(out, bytes) {
        Ok(_) -> 0" \
    "    var nb = byte_builder()
    nb.push_bytes(bytes)
    nb.push_bytes(\"{now_millis()}\".to_bytes())
    match write_bytes(out, nb.to_bytes()) {
        Ok(_) -> 0" -- \
    scripts/wasmsubset.sh "not the same bytes" \
    scripts/playtest.sh:build "not the same bytes"

control w-floor compiler/wasm32/src/main.gr \
    "} else if l.exports { 700 } else { 108 }" "} else if l.exports { 700 } else { 0 }" -- \
    scripts/wasmsubset.sh "past any depth"

control w-hop $B \
    "    if f.floor and o.hop_fn >= 0 {
        try hop(f, s, o, b)" \
    "    if f.floor and o.hop_fn >= 0 and false {
        try hop(f, s, o, b)" -- \
    scripts/wasmsubset.sh:stack "past any depth"

control w-small compiler/wasm32/src/main.gr \
    "} else if l.exports { 700 } else { 108 }" "} else if l.exports { 700 } else { 80 }" -- \
    scripts/wasmsubset.sh:stack "past any depth"

control w-tier compiler/wasm32/src/main.gr \
    "} else if l.exports { 700 } else { 108 }" "} else if l.exports { 700 } else { 100 }" -- \
    scripts/wasmsubset.sh:stack "under --no-liftoff, past any depth"

control w-deep-catch compiler/wasm32/src/main.gr \
    "        enc.try_table_catch(db, enc.BT_EMPTY, 0, 0)" "        enc.open(db, enc.OP_BLOCK, enc.BT_EMPTY)" -- \
    scripts/wasmsubset.sh:stack "past any depth"

control w-done compiler/rtwasm/src/deep.gr \
    "        if status != 1 {
            __wasm_trap()
        }" "" -- \
    scripts/wasmsubset.sh:stack "a trap in a deeper stack"

control w-seg-nondet compiler/s3/src/main.gr \
    "    lo.exports = exports
" "    lo.exports = exports
    lo.segment = lo.segment - now_millis() % 65536
" -- \
    scripts/wasmsubset.sh "not the same bytes"

control w-deep-words compiler/rtwasm/src/deep.gr \
    '            grasm_panic_msg(mem_static("stack overflow: recursion too deep"), mem_static_len("stack overflow: recursion too deep"))' \
    '            grasm_panic_msg(mem_static("stack overflow: no stack to hop to"), mem_static_len("stack overflow: no stack to hop to"))' -- \
    scripts/wasmsubset.sh:stack "without JSPI, past any depth"

control w-leb compiler/wasm32/src/enc.gr \
    "        if (x == 0 and lo.bit_and(64) == 0) or (x == -1 and lo.bit_and(64) != 0) {" \
    "        if (x == 0 and lo.bit_and(32) == 0) or (x == -1 and lo.bit_and(64) != 0) {" -- \
    scripts/encodetest_wasm.sh "DIFFERS"

control w-libm compiler/rtwasm/src/math.gr \
    "let S1 = -1.66666666666666324348e-01" "let S1 = -1.66666666666666352103e-01" -- \
    scripts/fuzz_wasmmath.sh "DIFFERS"

P=compiler/rtwasm/play/streams.gr
G=tools/playground/src/main.gr

control p-frame $P \
    "        f.offset(4).store_u32(code.bit_and(4294967295).as_u32())
        f.offset(8).store_u32(on.as_u32())" \
    "        f.offset(8).store_u32(code.bit_and(4294967295).as_u32())
        f.offset(4).store_u32(on.as_u32())" -- \
    scripts/playtest.sh:frame "not as the Rust module answers it"

control p-words $G \
    '                eprint("cannot replay: {why}")' '                eprint("cannot replay journal: {why}")' -- \
    scripts/playtest.sh:frame "not as the Rust module answers it"

control p-journal $G \
    "            tirinterp.run_tree_with(tree, SHOWN, none, stacks, room_left, kind == 3, jr)" \
    "            tirinterp.run_tree_with(tree, SHOWN, none, stacks, room_left, false, jr)" -- \
    scripts/playtest.sh:replay "not replayed alike"

control p-reset compiler/rtwasm/src/reset.gr \
    "        __wasm_memset(mem_addr(cells), 0, __wasm_cells_end() - cells)" "" -- \
    scripts/playtest.sh:session "state carried"

control p-stderr $P \
    "            append(mem_process(ERR_P), mem_process(ERR_N), mem_process(ERR_C), p, n)" "" -- \
    scripts/playtest.sh:frame "not as"

control p-hop compiler/rtwasm/src/stacks.gr \
    "    if wasm_stacks() != 1 {" "    if wasm_stacks() != 2 {" -- \
    scripts/playtest.sh:stack "under JSPI -- not as grasm run"

control p-nostack $G \
    "eval.STACKS_ROOM } else { eval.STACKS_NONE }" "eval.STACKS_ROOM } else { eval.STACKS_ROOM }" -- \
    scripts/playtest.sh:stack "without JSPI -- not the panic"

control p-dirty compiler/rtwasm/src/mem.gr \
    "                __wasm_memset(mem_addr(b), 0, len)
                return mem_addr(b)" "                return mem_addr(b)" -- \
    scripts/playtest.sh:stack "a mapping not zero"

echo
echo "controls: $controls   caught: $caught   missed: $((controls - caught))"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
# A floor: a control quietly dropped is a check quietly lost.
if [ "$controls" -lt 25 ]; then
    echo "fewer controls than the floor of 25"
    exit 1
fi
