#!/usr/bin/env bash
# The S5 gates must fail on a planted fault (docs/self-hosting.md §6.7).
#
# Each control copies `compiler/rt`, changes one thing in the copy, builds it
# with the Grasm backend as the gates build the real one, and runs the gate
# that should catch it against it (`RT_OBJ=`, `RT_LAYOUT=`), on the programs
# that reach the fault. The gate must fail, and for the reason the control
# names -- a gate that fails for some other reason has not shown it can see
# this one.
#
#   rc-twice        `dec` takes two from a count instead of one
#   float-short     a double is written one digit short
#   nan-min         `min` answering NaN for a signalling NaN, where Rust's
#                   answers the other operand (the fault `fuzz_rtmath` found)
#   strlen-off      a string value made one byte longer than its text
#   strlen-count    `len` one more than the characters of short ASCII text
#   panic-exit-1    a panic exits 1 instead of 101
#   layout-moved    a layout field moved: the inline length one byte on
#   fallback-gone   a moved function no longer exported, so programs take
#                   Rust's again
#   fallback-call   a moved function calling Rust's own instead of working
#   release-children the release walk not releasing a list's elements
#   release-leak    the release walk not freeing a heap string's buffer, which
#                   only `leaks` sees
#   release-arc     the release walk dropping a `bytes` value's last strong
#                   count without freeing the `Arc` itself
#   release-map     the walk not releasing a map's values
#   release-tuplekey the walk freeing a tuple key's slice but not the slices
#                   of the tuples inside it
#   release-struct  the walk not releasing a struct's boxed fields
#   release-instr   the walk not freeing a value list's inline strings
#   release-invals  the walk not taking apart the inline lists of values a
#                   value list's elements hold
#   release-env     the walk not releasing what an environment captured
#   release-opname  the walk not freeing a handler's operation names
#   alloc-live      a box given back without one fewer counted live
#   alloc-escaped   a thread's live count not handed to the process when the
#                   thread ends, which only the leak check at exit adds up
#   alloc-poison    the allocator taking the poisoned build for the ordinary
#                   one, and so handing a freed box out again
#   alloc-handover  the leak check not handing this side's balance to the
#                   Rust runtime's ledger before its check counts
#   map-hash        the hash's last multiplier one off
#   map-seed        the maps' seed not drawn from `getentropy`
#   map-tombstone   a deleted entry's bucket always made EMPTY, never DELETED
#   map-rehash      a table full of tombstones grown rather than rehashed in
#                   place
#   map-grow        a table for fifteen entries given sixteen buckets, not
#                   thirty-two
#   map-order       keys presented shortest first rather than in their order
#   eq-float        a struct's float fields compared as bits, so NaN equals
#                   itself
#   eq-mixed        a scalar list never equal to the same list of boxes
#   eq-fn           two closures equal only when they share one environment
#   cmp-float       floats ordered by their bits as integers, which puts
#                   -1.0 below -2.0
#   show-map        a map shown in the order of its buckets, not its keys
#   show-field      a struct's float field shown as the integer its bits are
#   show-hex        `bytes` shown in uppercase hex
#   struct-cow      a field written into a struct something else holds,
#                   without copying it first
#   struct-take     a field taken out of a struct something else holds
#   variant-field   a variant's every field read as its first
#   ref-generation  a released slot used again under the same generation, so
#                   a stale reference reads its next occupant
#   ref-drain       the leak check not releasing what the reference heap holds
#   handler-outer   an operation resolved to the outermost handler for its
#                   effect rather than the innermost
#   fn-name         a named function value made without its name
#   bytes-shared    a write landing in a buffer a window still shares
#   bytes-lossy     each invalid byte its own U+FFFD, not each sequence
#   builder-cow     an append to a builder something else holds, in place
#   list-cow        a write to a list something else holds, in place
#   list-stable     a sort that may swap equal keys
#   list-negzero    a list of floats summed from 0.0 rather than -0.0
#   range-incl      an inclusive range one element short
#   str-cow         an append to a string something else holds, in place
#   str-white       U+3000 not taken for whitespace
#   str-cr          a line's `\r` kept before its `\n`
#   str-flat        text with characters of more than a byte indexed by byte
#   case-table      a case table read with one hex digit wrong
#   case-sigma      a capital sigma lowercased as final where none follows
#   parse-tie       a decimal exactly halfway rounded to odd
#   parse-space     a number read without trimming the space around it
#   fixed-tie       `to_fixed` rounding an exact tie to even
#   int-saturate    a saturating product past the range held to the wrong end
#   task-rendezvous a rendezvous send that goes on before its value is taken
#   task-floor      a task's recursion checked against its thread's floor
#   task-wake       a scheduler taking wakes meant for another block
#   task-select     `chan_select` answering the last ready channel
#
# `RT_CONTROLS=<regex>` runs only the controls it matches. `RT_POISON=` as
# for `scripts/rtswap.sh`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
. "$(dirname "$0")/lib/link.sh"
s3_driver
link_driver
export S3 LINK GRASM

caught=0
missed=0
ran=0
declare -a MISSED=()

# Copies compiler/rt to $WORK/$1 and replaces the text $3 with $4 in its
# file $2 (exactly once), then builds the copy's object, $WORK/$1.o.
plant() {
    local name=$1 file=$2
    rm -rf "${WORK:?}/$name"
    cp -R compiler/rt "$WORK/$name"
    if ! python3 - "$WORK/$name/$file" "$3" "$4" <<'EOF'; then
import sys
path, old, new = sys.argv[1:4]
text = open(path).read()
if text.count(old) != 1:
    print(f"the control's text occurs {text.count(old)} times in {path}")
    sys.exit(1)
open(path, "w").write(text.replace(old, new))
EOF
        return 1
    fi
    "$GRASM" tir "$WORK/$name/src/main.gr" >"$WORK/$name.tir" 2>"$WORK/$name.log" &&
        "$S3" "$WORK/$name.tir" -o "$WORK/$name.o" --runtime >>"$WORK/$name.log" 2>&1
}

# Runs `$3...` (a gate, with the planted object in RT_OBJ) and requires it to
# fail with output matching `$2`.
expect() {
    local name=$1 why=$2
    shift 2
    ran=$((ran + 1))
    if RT_OBJ="$WORK/$name.o" "$@" >"$WORK/$name.out" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("$name: $* passed")
        echo "MISSED: $name ($*)"
    elif grep -qE "$why" "$WORK/$name.out"; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$WORK/$name.out" | head -1 | cut -c1-150)"
    else
        missed=$((missed + 1))
        MISSED+=("$name: $* failed, but not for its reason: $(tail -2 "$WORK/$name.out" | tr '\n' ' ' | cut -c1-200)")
        echo "MISSED: $name failed for another reason"
    fi
}

control() {
    [ -z "${RT_CONTROLS:-}" ] || [[ "$1" =~ $RT_CONTROLS ]]
}

if control rc-twice; then
    plant rc-twice src/val.gr 'v.atomic_add_i64(-1, Release) - 1 > 0' 'v.atomic_add_i64(-2, Release) - 2 > 0' ||
        { echo "error: rc-twice does not build"; exit 1; }
    expect rc-twice 'FAIL: ' env RT_S3=0 RT_PROGRAMS='^examples/' bash scripts/rtswap.sh
fi

if control float-short; then
    plant float-short src/fmt.gr '    let count = shortest(mant, minus, plus, exp, inclusive, digits, k)' \
        '    let full = shortest(mant, minus, plus, exp, inclusive, digits, k)
    let count = if full > 1 { full - 1 } else { full }' ||
        { echo "error: float-short does not build"; exit 1; }
    expect float-short 'the two runtimes differ' env RTFMT_CHUNKS=1 RTFMT_COUNT=2000 RTFMT_PY=100 bash scripts/fuzz_rtfmt.sh
fi

if control nan-min; then
    plant nan-min src/math.gr 'unsafe { fmin(quiet(a), quiet(b)) }' 'unsafe { fmin(a, b) }' ||
        { echo "error: nan-min does not build"; exit 1; }
    expect nan-min 'the two runtimes differ' env RTMATH_CHUNKS=1 RTMATH_COUNT=20000 bash scripts/fuzz_rtmath.sh
fi

if control strlen-off; then
    plant strlen-off src/numstr.gr 'v.offset(layout.STR_LEN).store_u8(n.as_u8())' 'v.offset(layout.STR_LEN).store_u8(n.as_u8() + 1)' ||
        { echo "error: strlen-off does not build"; exit 1; }
    expect strlen-off 'the two runtimes differ' env RTFMT_CHUNKS=1 RTFMT_COUNT=2000 RTFMT_PY=100 bash scripts/fuzz_rtfmt.sh
fi

if control strlen-count; then
    plant strlen-count src/strread.gr 'return if ascii(p, n) { n } else { count_chars(p, n) }' 'return if ascii(p, n) { n + 1 } else { count_chars(p, n) }' ||
        { echo "error: strlen-count does not build"; exit 1; }
    expect strlen-count 'FAIL: spec/conformance/(E-STR|L-LIB-STR)' env RT_S3=0 RT_POISON= RT_PROGRAMS='spec/conformance/(E-STR-CHARS|L-LIB-STR)\.gr' bash scripts/rtswap.sh
fi

if control panic-exit-1; then
    plant panic-exit-1 src/panic.gr 'pub fn div_zero() {
    unsafe { say(mem_static("division by zero"), mem_static_len("division by zero")) }
}' 'extern fn _exit(code: i32)

pub fn div_zero() {
    unsafe { _exit(1) }
}' || { echo "error: panic-exit-1 does not build"; exit 1; }
    expect panic-exit-1 'exit 101 \(Rust runtime\) vs 1' env RT_S3=0 RT_POISON= RT_PROGRAMS='spec/conformance/E-DIV-ZERO' bash scripts/rtswap.sh
fi

if control layout-moved; then
    plant layout-moved src/layout.gr 'pub let STR_LEN = 17' 'pub let STR_LEN = 18' ||
        { echo "error: layout-moved does not build"; exit 1; }
    expect layout-moved 'STR_LEN is 18 in layout.gr, 17 in the Rust runtime' env RT_POISON= RT_LAYOUT="$WORK/layout-moved/src/layout.gr" bash scripts/rtlayout.sh
    expect layout-moved 'the two runtimes differ' env RTFMT_CHUNKS=1 RTFMT_COUNT=2000 RTFMT_PY=100 bash scripts/fuzz_rtfmt.sh
fi

if control release-children; then
    plant release-children src/release.gr '        if to_zero(c) {
            push(stack, cap, pending, c)
        }' '        if to_zero(c) {
        }' || { echo "error: release-children does not build"; exit 1; }
    expect release-children 'the two runtimes differ' env RTREL_SEEDS=2 RTREL_ROUNDS=20 RTREL_LEAKS=0 bash scripts/fuzz_rtrelease.sh
fi

if control release-leak; then
    plant release-leak src/release.gr '            if v.offset(layout.STR_DISC).load_u8().as_i64() == layout.STR_HEAP {
                free_vec(v.offset(layout.STR_STRING))
            }' '' || { echo "error: release-leak does not build"; exit 1; }
    expect release-leak 'FAIL: f.swap seed 1: Process [0-9]+: [1-9][0-9]* leaks' env RTREL_SEEDS=1 RTREL_ROUNDS=20 RTREL_LEAKS=1 bash scripts/fuzz_rtrelease.sh
fi

if control release-arc; then
    plant release-arc src/release.gr '                if arc.offset(layout.ARC_WEAK).atomic_add_i64(-1, Release) == 1 {
                    sys.dealloc(arc)
                }' '' || { echo "error: release-arc does not build"; exit 1; }
    expect release-arc 'FAIL: f.swap seed 1: Process [0-9]+: [1-9][0-9]* leaks' env RTREL_SEEDS=1 RTREL_ROUNDS=20 RTREL_LEAKS=1 bash scripts/fuzz_rtrelease.sh
fi

if control release-map; then
    plant release-map src/release.gr '                        children(ctrl.offset(-(i + 1) * layout.MAP_ENTRY + layout.MAP_ENTRY_VAL), 1, stack, cap, pending)
' '' || { echo "error: release-map does not build"; exit 1; }
    expect release-map 'the two runtimes differ' env RTREL_SEEDS=2 RTREL_ROUNDS=20 RTREL_LEAKS=0 bash scripts/fuzz_rtrelease.sh
fi

if control release-tuplekey; then
    plant release-tuplekey src/release.gr '                } else if kw == layout.MAPKEY_TUPLE {
                    push(slices, cap, n, k.offset(layout.MAPKEY_TUPLE_PTR).load_addr())
                    push(slices, cap, n, mem_addr(k.offset(layout.MAPKEY_TUPLE_LEN).load_i64()))
                }' '                }' || { echo "error: release-tuplekey does not build"; exit 1; }
    expect release-tuplekey 'FAIL: f.swap seed 1: Process [0-9]+: [1-9][0-9]* leaks' env RTREL_SEEDS=1 RTREL_ROUNDS=20 RTREL_LEAKS=1 bash scripts/fuzz_rtrelease.sh
fi

if control release-struct; then
    plant release-struct src/release.gr '                if kind_at(kp, 1, nk, i) == layout.FK_BOXED {
                    children(at.offset(8 * i), 1, stack, cap, pending)
                }' '' || { echo "error: release-struct does not build"; exit 1; }
    expect release-struct 'the two runtimes differ' env RTREL_SEEDS=2 RTREL_ROUNDS=20 RTREL_LEAKS=0 bash scripts/fuzz_rtrelease.sh
fi

if control release-instr; then
    plant release-instr src/release.gr '                } else if k == layout.FK_INLIST or k == layout.FK_INSTR {
                    free_vec(f)' '                } else if k == layout.FK_INLIST {
                    free_vec(f)' || { echo "error: release-instr does not build"; exit 1; }
    expect release-instr 'FAIL: f.swap seed 1: Process [0-9]+: [1-9][0-9]* leaks' env RTREL_SEEDS=1 RTREL_ROUNDS=20 RTREL_LEAKS=1 bash scripts/fuzz_rtrelease.sh
fi

if control release-invals; then
    plant release-invals src/release.gr '                    push(work, wcap, wn, elems.offset(8 * i).load_addr())
                    push(work, wcap, wn, f.offset(layout.VEC_PTR).load_addr())
                    push(work, wcap, wn, mem_addr(f.offset(layout.VEC_LEN).load_i64()))
                    push(work, wcap, wn, mem_addr(f.offset(layout.VEC_CAP).load_i64()))' '' ||
        { echo "error: release-invals does not build"; exit 1; }
    expect release-invals 'the two runtimes differ' env RTREL_SEEDS=2 RTREL_ROUNDS=20 RTREL_LEAKS=0 RTREL_POISONED=0 bash scripts/fuzz_rtrelease.sh
fi

if control release-env; then
    plant release-env src/release.gr '                if fp.offset(i).load_u8().as_i64() != 0 and wp.offset(8 * i).load_i64() != 0 {' \
        '                if fp.offset(i).load_u8().as_i64() == 2 and wp.offset(8 * i).load_i64() != 0 {' ||
        { echo "error: release-env does not build"; exit 1; }
    expect release-env 'the two runtimes differ' env RTREL_SEEDS=2 RTREL_ROUNDS=20 RTREL_LEAKS=0 bash scripts/fuzz_rtrelease.sh
fi

if control release-opname; then
    plant release-opname src/release.gr '                free_vec(op.offset(i * layout.OP_SIZE + layout.OP_NAME))
' '' || { echo "error: release-opname does not build"; exit 1; }
    expect release-opname 'FAIL: f.swap seed 1: Process [0-9]+: [1-9][0-9]* leaks' env RTREL_SEEDS=1 RTREL_ROUNDS=20 RTREL_LEAKS=1 bash scripts/fuzz_rtrelease.sh
fi

if control alloc-live; then
    plant alloc-live src/alloc.gr '        l.offset(L_LIVE).store_i64(l.offset(L_LIVE).load_i64() - 1)
        if poisoned() {' '        if poisoned() {' || { echo "error: alloc-live does not build"; exit 1; }
    expect alloc-live 'the two runtimes differ' env RTREL_SEEDS=2 RTREL_ROUNDS=20 RTREL_LEAKS=0 RTREL_POISONED=0 bash scripts/fuzz_rtrelease.sh
fi

if control alloc-escaped; then
    plant alloc-escaped src/alloc.gr '        mem_process(ESCAPED).atomic_add_i64(b.offset(L_LIVE).load_i64(), AcqRel)
' '' || { echo "error: alloc-escaped does not build"; exit 1; }
    expect alloc-escaped 'the two runtimes differ' env RTREL_SEEDS=2 RTREL_ROUNDS=20 RTREL_LEAKS=0 RTREL_POISONED=0 bash scripts/fuzz_rtrelease.sh
fi

if control alloc-poison; then
    plant alloc-poison src/alloc.gr '        if known != 0 {
            return known == 2
        }' '        if known != 0 or known == 0 {
            return false
        }' || { echo "error: alloc-poison does not build"; exit 1; }
    expect alloc-poison 'differ under the poisoned runtime' env RTREL_SEEDS=1 RTREL_ROUNDS=20 RTREL_LEAKS=0 RTREL_POISONED=2 bash scripts/fuzz_rtrelease.sh
fi

if control alloc-handover; then
    plant alloc-handover src/leak.gr '            if stats {
                hand_over(made)
                hand_over(live - made)
            } else {
                hand_over(live)
            }' '' || { echo "error: alloc-handover does not build"; exit 1; }
    expect alloc-handover 'the two runtimes differ' env RTREL_SEEDS=2 RTREL_ROUNDS=20 RTREL_LEAKS=0 RTREL_POISONED=0 bash scripts/fuzz_rtrelease.sh
fi

if control map-hash; then
    plant map-hash src/maps.gr '    0x8ebc_6af0_9c88_c6e3' '    0x8ebc_6af0_9c88_c6e5' ||
        { echo "error: map-hash does not build"; exit 1; }
    expect map-hash 'pinned to .*: the two runtimes differ' env RTMAPS_SEEDS=2 RTMAPS_FREE=0 RTMAPS_OPS=300 bash scripts/fuzz_rtmaps.sh
fi

if control map-seed; then
    plant map-seed src/maps.gr '            mem_process(SEED).atomic_store_u64(at.load_u64(), Release)' '            mem_process(SEED).atomic_store_u64(at.load_u64() + 1, Release)' ||
        { echo "error: map-seed does not build"; exit 1; }
    expect map-seed 'pinned to .*: the two runtimes differ' env RTMAPS_SEEDS=2 RTMAPS_FREE=0 RTMAPS_OPS=300 bash scripts/fuzz_rtmaps.sh
fi

if control map-tombstone; then
    plant map-tombstone src/maps.gr '    if lead + trail < GROUP {' '    if lead + trail < GROUP + GROUP {' ||
        { echo "error: map-tombstone does not build"; exit 1; }
    expect map-tombstone 'pinned to .*: the two runtimes differ' env RTMAPS_SEEDS=4 RTMAPS_FREE=0 RTMAPS_OPS=1500 bash scripts/fuzz_rtmaps.sh
fi

if control map-rehash; then
    plant map-rehash src/maps.gr '    if want <= full / 2 {' '    if want <= full / 2 and false {' ||
        { echo "error: map-rehash does not build"; exit 1; }
    expect map-rehash 'pinned to .*: the two runtimes differ' env RTMAPS_SEEDS=10 RTMAPS_FREE=0 RTMAPS_OPS=1500 bash scripts/fuzz_rtmaps.sh
fi

if control map-grow; then
    plant map-grow src/maps.gr '    if cap < 15 {' '    if cap < 16 {' ||
        { echo "error: map-grow does not build"; exit 1; }
    expect map-grow 'pinned to .*: the two runtimes differ' env RTMAPS_SEEDS=4 RTMAPS_FREE=0 RTMAPS_OPS=1500 bash scripts/fuzz_rtmaps.sh
fi

if control map-order; then
    plant map-order src/keys.gr '                out = if c < 0 { -1 } else if c > 0 { 1 } else if nx < ny { -1 } else if nx > ny { 1 } else { 0 }' '                out = if nx < ny { -1 } else if nx > ny { 1 } else if c < 0 { -1 } else if c > 0 { 1 } else { 0 }' ||
        { echo "error: map-order does not build"; exit 1; }
    expect map-order 'seed 100[0-9]: the two runtimes differ' env RTMAPS_SEEDS=0 RTMAPS_FREE=3 RTMAPS_OPS=1500 bash scripts/fuzz_rtmaps.sh
fi

if control eq-float; then
    plant eq-float src/equal.gr '                } else if k == layout.FK_F64 {
                    if not same_float(a, b) {
                        return false
                    }
                } else if a != b {' '                } else if a != b {' ||
        { echo "error: eq-float does not build"; exit 1; }
    expect eq-float 'the two runtimes differ' env RTEQ_SEEDS=5 RTEQ_ROUNDS=300 bash scripts/fuzz_rteq.sh
fi

if control eq-mixed; then
    plant eq-mixed src/equal.gr '            if tx == layout.TAG_LIST_SCALAR and ty == layout.TAG_LIST {
                return scalars_vs_boxes(x, y)
            }' '' || { echo "error: eq-mixed does not build"; exit 1; }
    expect eq-mixed 'the two runtimes differ' env RTEQ_SEEDS=5 RTEQ_ROUNDS=300 bash scripts/fuzz_rteq.sh
fi

if control eq-fn; then
    plant eq-fn src/equal.gr '            if e1.as_i64() == 0 or e2.as_i64() == 0 {
                return false
            }' '            if e1.as_i64() == 0 or e2.as_i64() != 0 {
                return false
            }' || { echo "error: eq-fn does not build"; exit 1; }
    expect eq-fn 'the two runtimes differ' env RTEQ_SEEDS=5 RTEQ_ROUNDS=300 bash scripts/fuzz_rteq.sh
fi

if control cmp-float; then
    plant cmp-float src/equal.gr '                out = three(total_key(word(x)), total_key(word(y)))' '                out = three(word(x), word(y))' ||
        { echo "error: cmp-float does not build"; exit 1; }
    expect cmp-float 'the two runtimes differ' env RTEQ_SEEDS=5 RTEQ_ROUNDS=300 bash scripts/fuzz_rteq.sh
fi

if control show-map; then
    plant show-map src/keys.gr '                let take_x = y >= hi or (x < mid and key_cmp(unsafe { a.offset(8 * x).load_addr() }, unsafe { a.offset(8 * y).load_addr() }) <= 0)' '                let take_x = y >= hi or x < mid' ||
        { echo "error: show-map does not build"; exit 1; }
    expect show-map 'the two runtimes differ' env RTSHOW_SEEDS=3 RTSHOW_ROUNDS=50 bash scripts/fuzz_rtshow.sh
fi

if control show-field; then
    plant show-field src/display.gr '    if kind == layout.FK_F64 {
        put_float(b, bits)
    } else if kind == layout.FK_BOOL {' '    if kind == layout.FK_BOOL {' ||
        { echo "error: show-field does not build"; exit 1; }
    expect show-field 'the two runtimes differ' env RTSHOW_SEEDS=3 RTSHOW_ROUNDS=50 bash scripts/fuzz_rtshow.sh
fi

if control show-hex; then
    plant show-hex src/display.gr '    if d < 10 { 48 + d } else { 87 + d }' '    if d < 10 { 48 + d } else { 55 + d }' ||
        { echo "error: show-hex does not build"; exit 1; }
    expect show-hex 'the two runtimes differ' env RTSHOW_SEEDS=3 RTSHOW_ROUNDS=50 bash scripts/fuzz_rtshow.sh
fi

if control struct-cow; then
    plant struct-cow src/structs.gr '        if s.atomic_load_i64(Relaxed) == 1 {
            return s
        }
    }
    if tag(s) != layout.TAG_STRUCT {
        unsafe { say(mem_static("set_field on non-struct"), mem_static_len("set_field on non-struct")) }
    }' '        if s.atomic_load_i64(Relaxed) >= 1 {
            return s
        }
    }
    if tag(s) != layout.TAG_STRUCT {
        unsafe { say(mem_static("set_field on non-struct"), mem_static_len("set_field on non-struct")) }
    }' || { echo "error: struct-cow does not build"; exit 1; }
    expect struct-cow 'the two runtimes differ' env RTSTRUCTS_SEEDS=3 RTSTRUCTS_ROUNDS=100 bash scripts/fuzz_rtstructs.sh
fi

if control struct-take; then
    plant struct-take src/structs.gr '        if s.atomic_load_i64(Relaxed) != 1 {
            return mem_addr(0)
        }' '        if s.atomic_load_i64(Relaxed) < 1 {
            return mem_addr(0)
        }' || { echo "error: struct-take does not build"; exit 1; }
    expect struct-take 'the two runtimes differ' env RTSTRUCTS_SEEDS=3 RTSTRUCTS_ROUNDS=100 bash scripts/fuzz_rtstructs.sh
fi

if control variant-field; then
    plant variant-field src/structs.gr '    let f = unsafe { vfields(v).offset(8 * i).load_addr() }' '    let f = unsafe { vfields(v).load_addr() }' ||
        { echo "error: variant-field does not build"; exit 1; }
    expect variant-field 'the two runtimes differ' env RTSTRUCTS_SEEDS=3 RTSTRUCTS_ROUNDS=100 bash scripts/fuzz_rtstructs.sh
fi

if control ref-generation; then
    plant ref-generation src/refs.gr '            gen(i).store_i64(g + 1)' '            gen(i).store_i64(g)' ||
        { echo "error: ref-generation does not build"; exit 1; }
    expect ref-generation 'the two runtimes differ' env RTREFS_SEEDS=4 RTREFS_ROUNDS=200 bash scripts/fuzz_rtrefs.sh
fi

if control ref-drain; then
    plant ref-drain src/leak.gr '            refs.drain()
' '' || { echo "error: ref-drain does not build"; exit 1; }
    expect ref-drain 'the two runtimes differ' env RTREFS_SEEDS=2 RTREFS_ROUNDS=200 bash scripts/fuzz_rtrefs.sh
fi

if control handler-outer; then
    plant handler-outer src/funcs.gr '        var i = mem_tls(NFRAMES).load_i64() - 1
        while i >= 0 {
            let f = frame(i)' '        var i = 0
        while i < mem_tls(NFRAMES).load_i64() {
            let f = frame(i)' || { echo "error: handler-outer does not build"; exit 1; }
    python3 - "$WORK/handler-outer/src/funcs.gr" <<'EOF' || { echo "error: handler-outer does not plant"; exit 1; }
import sys
p = sys.argv[1]
s = open(p).read()
old = """                return mem_addr(0)
            }
            i -= 1
        }"""
assert s.count(old) == 1
open(p, "w").write(s.replace(old, """                return mem_addr(0)
            }
            i += 1
        }"""))
EOF
    "$GRASM" tir "$WORK/handler-outer/src/main.gr" >"$WORK/handler-outer.tir" 2>"$WORK/handler-outer.log" &&
        "$S3" "$WORK/handler-outer.tir" -o "$WORK/handler-outer.o" --runtime >>"$WORK/handler-outer.log" 2>&1 ||
        { echo "error: handler-outer does not build"; exit 1; }
    expect handler-outer 'FAIL: spec/conformance/E-HANDLER-INNERMOST' env RT_S3=0 RT_POISON= RT_PROGRAMS='spec/conformance/E-HANDLER-INNERMOST\.gr' bash scripts/rtswap.sh
fi

if control fn-name; then
    plant fn-name src/funcs.gr '        if nl > 0 {
            string_at(name, np, nl)' '        if nl < 0 {
            string_at(name, np, nl)' || { echo "error: fn-name does not build"; exit 1; }
    expect fn-name 'the two runtimes differ' env RTSHOW_SEEDS=2 RTSHOW_ROUNDS=20 bash scripts/fuzz_rtshow.sh
fi

if control bytes-shared; then
    plant bytes-shared src/bytes.gr '        v.offset(layout.BYTES_PTR).load_i64() == data.offset(layout.VEC_PTR).load_i64() and v.offset(layout.BYTES_LEN).load_i64() == data.offset(layout.VEC_LEN).load_i64() and arc.offset(layout.ARC_STRONG).atomic_load_i64(Acquire) == 1 and arc.offset(layout.ARC_WEAK).atomic_load_i64(Acquire) == 1' '        arc.offset(layout.ARC_WEAK).atomic_load_i64(Acquire) == 1' ||
        { echo "error: bytes-shared does not build"; exit 1; }
    expect bytes-shared 'the two runtimes differ' env RTBYTES_SEEDS=3 RTBYTES_ROUNDS=100 bash scripts/fuzz_rtbytes.sh
fi

if control bytes-lossy; then
    plant bytes-lossy src/bytes.gr '        if not ok {
            return 1
        }
        if not cont(at(p, n, i + 2)) {
            return 2
        }
        whole = true
        return 3' '        if not ok {
            return 1
        }
        if not cont(at(p, n, i + 2)) {
            return 1
        }
        whole = true
        return 3' || { echo "error: bytes-lossy does not build"; exit 1; }
    expect bytes-lossy 'the two runtimes differ' env RTBYTES_SEEDS=3 RTBYTES_ROUNDS=100 bash scripts/fuzz_rtbytes.sh
fi

if control builder-cow; then
    plant builder-cow src/bytes.gr '        if b.atomic_load_i64(Relaxed) == 1 {
            return b
        }' '        if b.atomic_load_i64(Relaxed) >= 1 {
            return b
        }' || { echo "error: builder-cow does not build"; exit 1; }
    expect builder-cow 'the two runtimes differ' env RTBYTES_SEEDS=3 RTBYTES_ROUNDS=100 bash scripts/fuzz_rtbytes.sh
fi

if control list-cow; then
    plant list-cow src/lists.gr '    if rc(l) == 1 {
        return l
    }
    let t = tag(l)
    var out' '    if rc(l) >= 1 {
        return l
    }
    let t = tag(l)
    var out' || { echo "error: list-cow does not build"; exit 1; }
    expect list-cow 'the two runtimes differ' env RTLISTS_SEEDS=3 RTLISTS_ROUNDS=50 bash scripts/fuzz_rtlists.sh
fi

if control list-stable; then
    plant list-stable src/lists.gr 'if compare(mode, y, x, sk, keys) < 0 {' 'if compare(mode, y, x, sk, keys) <= 0 {' ||
        { echo "error: list-stable does not build"; exit 1; }
    expect list-stable 'the two runtimes differ' env RTLISTS_SEEDS=3 RTLISTS_ROUNDS=50 bash scripts/fuzz_rtlists.sh
fi

if control list-negzero; then
    plant list-negzero src/lists.gr '            var s = 1.shl(63).from_bits()' '            var s = 0.0' ||
        { echo "error: list-negzero does not build"; exit 1; }
    expect list-negzero 'the two runtimes differ' env RTLISTS_SEEDS=3 RTLISTS_ROUNDS=50 bash scripts/fuzz_rtlists.sh
fi

if control range-incl; then
    plant range-incl src/lists.gr 'return if d >= max.as_u64() { max } else { d.as_i64() + 1 }' 'return if d >= max.as_u64() { max } else { d.as_i64() }' ||
        { echo "error: range-incl does not build"; exit 1; }
    expect range-incl 'the two runtimes differ' env RTLISTS_SEEDS=3 RTLISTS_ROUNDS=50 bash scripts/fuzz_rtlists.sh
fi

if control str-cow; then
    plant str-cow src/strings.gr '    if rc(a) == 1 {
        push_text(a, p, n)' '    if rc(a) >= 1 {
        push_text(a, p, n)' || { echo "error: str-cow does not build"; exit 1; }
    expect str-cow 'the two runtimes differ' env RTSTRINGS_SEEDS=3 RTSTRINGS_ROUNDS=30 bash scripts/fuzz_rtstrings.sh
fi

if control str-white; then
    plant str-white src/strings.gr ' or c == 8287 or c == 12288' ' or c == 8287' ||
        { echo "error: str-white does not build"; exit 1; }
    expect str-white 'the two runtimes differ' env RTSTRINGS_SEEDS=3 RTSTRINGS_ROUNDS=30 bash scripts/fuzz_rtstrings.sh
fi

if control str-cr; then
    plant str-cr src/strings.gr '            if stop > from and byte(p, stop - 1) == 13 {' '            if stop > from and byte(p, stop - 1) == 14 {' ||
        { echo "error: str-cr does not build"; exit 1; }
    expect str-cr 'the two runtimes differ' env RTSTRINGS_SEEDS=3 RTSTRINGS_ROUNDS=30 bash scripts/fuzz_rtstrings.sh
fi

if control str-flat; then
    plant str-flat src/strings.gr '        if n < LEN_CACHE_MIN {
            return strread.ascii(p, n)
        }' '        if n < LEN_CACHE_MIN {
            return true
        }' || { echo "error: str-flat does not build"; exit 1; }
    expect str-flat 'the two runtimes differ' env RTSTRINGS_SEEDS=3 RTSTRINGS_ROUNDS=30 bash scripts/fuzz_rtstrings.sh
fi

if control case-table; then
    plant case-table src/case.gr '    if c <= 57 { c - 48 } else { c - 87 }' '    if c <= 57 { c - 48 } else if c == 102 { 14 } else { c - 87 }' ||
        { echo "error: case-table does not build"; exit 1; }
    expect case-table 'the two runtimes differ' bash scripts/fuzz_rtcase.sh
fi

if control case-sigma; then
    plant case-sigma src/case.gr '    if not before {
        return false
    }' '    if not before {
        return true
    }' || { echo "error: case-sigma does not build"; exit 1; }
    expect case-sigma 'the two runtimes differ' env RTSTRINGS_SEEDS=3 RTSTRINGS_ROUNDS=30 bash scripts/fuzz_rtstrings.sh
fi

if control parse-tie; then
    plant parse-tie src/numparse.gr '(dp != 0 and dig(d, dp - 1).bit_and(1) != 0)' '(dp != 0 and dig(d, dp - 1).bit_and(1) == 0)' ||
        { echo "error: parse-tie does not build"; exit 1; }
    expect parse-tie 'the two runtimes differ' env RTPARSE_SEEDS=3 RTPARSE_ROUNDS=100 bash scripts/fuzz_rtparse.sh
fi

if control parse-space; then
    plant parse-space src/numparse.gr '    let a = strings.lead(p, all)' '    let a = 0' ||
        { echo "error: parse-space does not build"; exit 1; }
    expect parse-space 'the two runtimes differ' env RTPARSE_SEEDS=3 RTPARSE_ROUNDS=100 bash scripts/fuzz_rtparse.sh
fi

if control fixed-tie; then
    plant fixed-tie src/numparse.gr '        var tie = byte(wp, wn - 25) == 53' '        var tie = false' ||
        { echo "error: fixed-tie does not build"; exit 1; }
    expect fixed-tie 'the two runtimes differ' env RTPARSE_SEEDS=3 RTPARSE_ROUNDS=100 bash scripts/fuzz_rtparse.sh
fi

if control int-saturate; then
    plant int-saturate src/numparse.gr '            over = if (a < 0) != (b < 0) { -1 } else { 1 }' '            over = 1' ||
        { echo "error: int-saturate does not build"; exit 1; }
    expect int-saturate 'the two runtimes differ' env RTPARSE_SEEDS=3 RTPARSE_ROUNDS=100 bash scripts/fuzz_rtparse.sh
fi

if control task-rendezvous; then
    plant task-rendezvous src/tasks.gr '            ticket = sent - 1
            return 2' '            ticket = sent - 1
            return 1' || { echo "error: task-rendezvous does not build"; exit 1; }
    expect task-rendezvous 'differs?' env RTTASKS_SEEDS=3 RTTASKS_ROUNDS=10 bash scripts/fuzz_rttasks.sh
fi

if control task-floor; then
    plant task-floor src/fiber.gr '        let old = cell.load_i64()
        cell.store_i64(f)' '        let old = cell.load_i64()
        cell.store_i64(old)' || { echo "error: task-floor does not build"; exit 1; }
    expect task-floor 'differs?' env RTTASKS_SEEDS=3 RTTASKS_ROUNDS=10 bash scripts/fuzz_rttasks.sh
fi

if control task-wake; then
    plant task-wake src/tasks.gr '        if wb == b {' '        if true {' ||
        { echo "error: task-wake does not build"; exit 1; }
    expect task-wake 'differs?' env RTTASKS_SEEDS=3 RTTASKS_ROUNDS=10 bash scripts/fuzz_rttasks.sh
fi

if control task-select; then
    plant task-select src/tasks.gr '            if q_len(c.offset(C_BUF).load_addr()) > 0 or c.offset(C_CLOSED).load_i64() != 0 {
                return i
            }' '            if (q_len(c.offset(C_BUF).load_addr()) > 0 or c.offset(C_CLOSED).load_i64() != 0) and i == n - 1 {
                return i
            }' || { echo "error: task-select does not build"; exit 1; }
    expect task-select 'differs?' env RTTASKS_SEEDS=3 RTTASKS_ROUNDS=10 bash scripts/fuzz_rttasks.sh
fi

if control fallback-gone; then
    plant fallback-gone src/math.gr 'export fn grt_f64_sqrt(' 'fn grt_f64_sqrt(' ||
        { echo "error: fallback-gone does not build"; exit 1; }
    expect fallback-gone 'says f64_sqrt has moved, but the Grasm runtime does not export' env RT_S3=0 RT_POISON= RT_PROGRAMS='^examples/hello' bash scripts/rtswap.sh
fi

if control fallback-call; then
    plant fallback-call src/math.gr 'export fn grt_f64_floor(v: f64) -> f64 {
    unsafe { floor(v) }
}' 'extern fn grasm_f64_floor(x: f64) -> f64

export fn grt_f64_floor(v: f64) -> f64 {
    unsafe { grasm_f64_floor(v) }
}' || { echo "error: fallback-call does not build"; exit 1; }
    expect fallback-call "calls the Rust runtime's _grasm_f64_floor" env RT_S3=0 RT_POISON= RT_PROGRAMS='^examples/hello' bash scripts/rtswap.sh
fi

echo
echo "checks of planted faults: $ran   caught: $caught   missed: $missed"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
[ "$ran" -ge 1 ] && [ "$caught" -eq "$ran" ]
