#!/usr/bin/env bash
# The linker gates' controls (docs/self-hosting.md, stage S4): each plants one
# fault in a copy of compiler/link's source -- or of codesig's, which signs
# its output -- builds that linker, and runs `scripts/linktest.sh` and
# `scripts/linkdiff.sh` with it on a small corner of the corpus. Every gate a
# control names must fail, and fail for the reason it names:
#
#   branch26    every BRANCH26 one instruction past its target
#               -> linktest (the programs run differently), linkdiff (bytes)
#   pageoff12   a load or store's PAGEOFF12 scaled by the wrong size
#               -> linkdiff (bytes). Not linktest here: the three loads it
#               changes in hello are on paths hello never runs
#   gotbind     each GOT entry and bound pointer bound to the next import
#               -> linktest, linkdiff (dyld_info -fixups)
#   sigpage     the signature's hash of the second page wrong
#               -> linktest (codesign -v --strict)
#   dropexport  dead-strip not rooting -exported_symbol, so an `export fn`
#               nothing calls is dropped
#               -> linktest, linkdiff (the export fixture)
#   tlvbind     thread-local descriptors not bound to __tlv_bootstrap
#               -> linkdiff (dyld_info -fixups on the thread-local
#               fixture). Not linktest: dyld writes its own thunk into
#               every descriptor of an image it loads, so a program runs
#               the same with the bind missing; only the image is wrong
#
# First the unmutated linker passes both gates on the same programs, so a
# failure below is the fault's and not the corner's.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The corner: one program, from a directory small enough to collect fast.
export DIRS=examples
export LINK_PROGRAMS='^examples/hello\.gr$'
export GRASM_NO_BUILD=1
export GRASM

# One S3 driver for every run.
S3="$WORK/s3"
if ! "$GRASM" build compiler/s3/src/main.gr --native -o "$S3" >"$WORK/s3-build.log" 2>&1; then
    echo "error: compiler/s3 does not build"
    exit 1
fi
export S3

controls=0
caught=0
baseline=0
declare -a MISSED=()

# The linker's source and the packages it depends on, copied where their
# relative paths still hold.
copy_source() {
    rm -rf "$WORK/src"
    mkdir -p "$WORK/src/compiler" "$WORK/src/packages"
    cp -R compiler/link "$WORK/src/compiler/"
    local p
    for p in codesig exporttrie crypto_hash machoread sha1 sha512; do
        cp -R "packages/$p" "$WORK/src/packages/"
    done
}

# run_gate NAME LINKER GATE: the gate's exit status; its log at $WORK/NAME-GATE.log.
run_gate() {
    local name=$1 linker=$2 gate=$3
    LINK="$linker" "$gate" >"$WORK/$name-$(basename "$gate" .sh).log" 2>&1
}

copy_source
if ! "$GRASM" build "$WORK/src/compiler/link/src/main.gr" --native -o "$WORK/link-clean" >"$WORK/build-clean.log" 2>&1; then
    echo "error: compiler/link does not build"
    exit 1
fi
for gate in scripts/linktest.sh scripts/linkdiff.sh; do
    if run_gate clean "$WORK/link-clean" "$gate"; then
        baseline=$((baseline + 1))
        echo "baseline: $(basename "$gate" .sh) passes unmutated"
    else
        echo "baseline: $(basename "$gate" .sh) FAILS unmutated; the controls below would prove nothing"
        tail -8 "$WORK/clean-$(basename "$gate" .sh).log" | sed 's/^/    /'
    fi
done

# control NAME FILE OLD NEW -- GATE SYMPTOM ...
control() {
    local name=$1 file=$2 old=$3 new=$4
    shift 5
    copy_source
    if ! python3 scripts/s3_mutate.py "$WORK/src/$file" "$old" "$new"; then
        MISSED+=("$name: the mutation does not apply")
        return
    fi
    local linker="$WORK/link-$name"
    if ! "$GRASM" build "$WORK/src/compiler/link/src/main.gr" --native -o "$linker" >"$WORK/build-$name.log" 2>&1; then
        MISSED+=("$name: the mutated linker does not build")
        sed 's/^/    /' "$WORK/build-$name.log" | head -5
        return
    fi
    while [ $# -ge 2 ]; do
        local gate=$1 symptom=$2
        shift 2
        controls=$((controls + 1))
        run_gate "$name" "$linker" "$gate"
        local code=$?
        local log="$WORK/$name-$(basename "$gate" .sh).log"
        if [ "$code" != 0 ] && grep -cF -- "$symptom" "$log" >/dev/null; then
            caught=$((caught + 1))
            echo "caught: $name by $(basename "$gate" .sh) (exit $code; \"$symptom\")"
        elif [ "$code" != 0 ] && grep -ciE -- "timed out after" "$log" >/dev/null; then
            caught=$((caught + 1))
            echo "caught: $name by $(basename "$gate" .sh) (exit $code; a program ran past the limit)"
        elif [ "$code" != 0 ]; then
            MISSED+=("$name: $(basename "$gate" .sh) failed (exit $code) but not with \"$symptom\"")
            tail -5 "$log" | sed 's/^/    /'
        else
            MISSED+=("$name: $(basename "$gate" .sh) passed with the fault in place")
        fi
    done
}

control branch26 compiler/link/src/apply.gr \
    "delta.shr(2).bit_and(0x3ffffff)" "(delta.shr(2) + 1).bit_and(0x3ffffff)" -- \
    scripts/linktest.sh "FAIL: examples/hello.gr" \
    scripts/linkdiff.sh "not the bytes the native build shipped"

control pageoff12 compiler/link/src/apply.gr \
    "var shift = insn.ushr(30).bit_and(3)" "var shift = insn.ushr(30).bit_and(1)" -- \
    scripts/linkdiff.sh "not the bytes the native build shipped"

control gotbind compiler/link/src/image.gr \
    "v = l.g_import[fx.target[i]].bit_and(0xffffff)" \
    "v = ((l.g_import[fx.target[i]] + 1) % l.imp_g.len()).bit_and(0xffffff)" -- \
    scripts/linktest.sh "FAIL: examples/hello.gr" \
    scripts/linkdiff.sh "dyld_info -fixups -exports differ"

control sigpage packages/codesig/src/main.gr \
    "crypto_hash.sha256_of(image.slice(k * 4096, to))" \
    "crypto_hash.sha256_of(image.slice(k * 4096 + (if k == 1 { 1 } else { 0 }), to))" -- \
    scripts/linktest.sh "codesign -v --strict refuses it"

control dropexport compiler/link/src/deadstrip.gr \
    "    for name in l.exp_list {" "    for name in l.exp_list.slice(0, 0) {" -- \
    scripts/linktest.sh "fixture export" \
    scripts/linkdiff.sh "fixture export"

control tlvbind compiler/link/src/apply.gr \
    "                    if k == state.T_IMPORT {
                        fx.addr.push(pc)" \
    "                    if k == state.T_IMPORT and l.s_flags[si].bit_and(255) == 0x13 {
                        continue
                    } else if k == state.T_IMPORT {
                        fx.addr.push(pc)" -- \
    scripts/linkdiff.sh "fixture tlv — dyld_info -fixups -exports differ"

echo
echo "baseline: $baseline of 2   controls: $controls   caught: $caught"
if [ ${#MISSED[@]} -gt 0 ]; then
    echo
    echo "NOT CAUGHT:"
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
[ "$baseline" = 2 ] && [ "$controls" -gt 0 ] && [ "$caught" = "$controls" ]
