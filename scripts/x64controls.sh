#!/usr/bin/env bash
# Mutation controls for the x86-64 target's gates (docs/self-hosting.md §6.16,
# stage S7): a gate that cannot fail checks nothing.
#
# Each control copies `compiler/` aside, plants one fault in the x86-64
# target (`scripts/s3_mutate.py`, which refuses a mutation that matches
# nothing), builds that broken driver, and runs the gate named for it with
# `S3=` pointing at it. The gate must fail, and fail for the planted reason:
# its log must show the symptom that fault produces.
#
#   opcode-bit   `add`'s register form is encoded with its opcode one bit off
#                (03, `add` the other way round, for 01) -> encodetest_x64
#   rex-r        the REX prefix's R bit, the fourth bit of the ModRM `reg`
#                field, is written where W goes -> encodetest_x64
#   imm8-edge    an immediate of 128 is taken to fit in 8 bits, so it is
#                written as -128 -> encodetest_x64
#   sib          a base of rsp or r12 gets no SIB byte, so the address means
#                another register -> encodetest_x64
#   disp-rbp     a displacement of 0 from rbp or r13 is left out, which the
#                processor reads as RIP-relative -> encodetest_x64
#   lock-order   a 16-bit locked instruction puts `lock` before the
#                operand-size prefix, the other order to the assembler's
#                -> encodetest_x64
#   jmp-rel      a short branch's displacement is written one byte long
#                -> encodetest_x64
#   reloc-kind   the RIP-relative address of a symbol asks for a GOT load's
#                relocation -> encodetest_x64 (the relocation check)
#   coverage     the target says it can emit a form the encoder has no cases
#                for -> encodetest_x64 (the coverage check)
#
# Step three's selection, allocation, frame and object (§6.16):
#
#   x-cc-map     the integer condition `slt` maps to `le`
#                -> encodetest_x64 (the rule table's check of the map)
#                -> xs3subset, with the table's checks skipped, by the output
#   x-fcc-swap   `<` and `<=` on doubles compare without swapping the
#                operands, so they read as `>` and `>=`
#                -> encodetest_x64 (the comparisons evaluated by IEEE 754)
#   x-flags      a `not` between `bsr` and the `cmov` reading its flags
#                -> encodetest_x64 (no flag setter between a comparison and
#                its reader)
#   x-pop-order  the epilogue pops the callee-saved registers in the order
#                the prologue pushed them -> xs3subset; and fuzz_x64 (fuzz_s3
#                and one generator on the s3 engine, S3_TARGET=x86_64), which
#                would pass were the target switch not reaching the backend
#   x-clobber    the target description hands out rcx, which a call changes,
#                to values living across calls -> xs3subset (the allocation
#                checker refuses every function that keeps one there)
#   x-scratch    r11, a reload's scratch register, is also handed out to
#                values -> ra_check_x64 (the checker, under pressure)
#   x-reloc-off  every relocation is recorded one byte short of its field
#                -> objtest_x64 (relocations against the instructions)
#   x-line       every line-table row starts a byte after its instruction
#                -> objtest_x64 (a literal's load maps to the wrong line)
#   x-nondet     the driver writes the clock into every object
#                -> objtest_x64 (two compilations differ)
#
# Step four's runtime and linker:
#
#   x-stub       a stub jumps through the GOT slot after its own
#                -> xrtswap (calls into libSystem land in the wrong function)
#   x-tlv-lea    a GOT or thread-local load of something defined in the
#                image is left a `movq` rather than made a `leaq`, so it reads
#                the target's first word as its address (the runtime's
#                thread-local descriptors)
#                -> xrtswap
#   x-ctx-switch the context switch restores `rsp` from the word after the
#                one it saved it in -> xrtswap (a concurrent program, whose
#                tasks run on fibers, fails)
#
# The linker against ld's x86-64 output (linkdiff_x64):
#
#   x-ld-sign    an x86-64 executable is signed, which ld does not do
#                -> linkdiff_x64 (the load commands)
#   x-ld-room    the room ld leaves after an x86-64 image's load commands is
#                taken as arm64's 32 bytes, not 48 -> linkdiff_x64 (a program
#                whose __TEXT then starts a page early)
#   x-ld-pages   the chained fixups describe every page of a segment, not up
#                to the last with a fixup -> linkdiff_x64
#   x-ld-tail    the empty atom a label at its section's end makes has no
#                function start -> linkdiff_x64 (the whole runtime)
#   x-ld-got     X86_64_RELOC_GOT to something defined here is made a `leaq`
#                like a GOT load -> linkdiff_x64 (the whole runtime)
#   x-ld-dwarf   the compact encoding that defers to DWARF is taken as
#                arm64's -> linkdiff_x64 (the whole runtime's __eh_frame)
#   x-ld-tlvalign the thread-local template's sections keep their own
#                alignments -> linkdiff_x64 (the tlv fixture)
#   x-ld-lsda    an FDE's LSDA field, which x86-64's objects leave without a
#                relocation, is rewritten 4 bytes off -> linkdiff_x64
#   x-ld-strip   a stripped x86-64 image's __LINKEDIT is sized in its 4 KiB
#                pages, where strip uses 16 KiB -> linkdiff_x64 (byte for
#                byte against ld and strip)
#
# The gates that compile the corpus stop at the first failure
# (`S3_STOP_AT_FIRST=1`). `S3_RT_X64=` is handed to them as given.
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

# control NAME FILE OLD NEW -- GATE SYMPTOM [EXTRA] [GATE SYMPTOM EXTRA] ...
control() {
    local name=$1 file=$2 old=$3 new=$4
    shift 5
    rm -rf "$WORK/src"
    mkdir -p "$WORK/src"
    cp -R compiler "$WORK/src/"
    # The linker's packages, read in place.
    ln -s "$PWD/packages" "$WORK/src/packages"
    if ! python3 scripts/s3_mutate.py "$WORK/src/$file" "$old" "$new"; then
        MISSED+=("$name: the mutation does not apply")
        return
    fi
    # The driver and the linker, both from the copy: a gate given `S3=` and
    # `LINK=` tests these rather than building its own from the tree, so a
    # fault planted in either is the one under test.
    local driver="$WORK/s3-$name"
    local linker="$WORK/link-$name"
    if ! "$GRASM" build "$WORK/src/compiler/s3/src/main.gr" --native -o "$driver" >"$WORK/build-$name.log" 2>&1 ||
        ! "$GRASM" build "$WORK/src/compiler/link/src/main.gr" --native -o "$linker" >>"$WORK/build-$name.log" 2>&1; then
        MISSED+=("$name: the mutated backend or linker does not build")
        sed 's/^/    /' "$WORK/build-$name.log" | head -5
        return
    fi
    while [ $# -ge 2 ]; do
        local gate=$1 symptom=$2 extra=${3:--}
        shift 2
        [ $# -ge 1 ] && shift
        [ "$extra" = "-" ] && extra=""
        controls=$((controls + 1))
        # A gate may start with VAR=value words, set for it alone.
        local -a cmd
        read -ra cmd <<<"$gate"
        gate=${cmd[${#cmd[@]} - 1]}
        local log="$WORK/$name-$(basename "$gate" .sh).log"
        S3="$driver" LINK="$linker" S3_EXTRA="$extra" S3_STOP_AT_FIRST=1 env "${cmd[@]}" >"$log" 2>&1
        local code=$?
        if [ "$code" != 0 ] && grep -cF -- "$symptom" "$log" >/dev/null; then
            caught=$((caught + 1))
            echo "caught: $name by $(basename "$gate" .sh) (exit $code; \"$symptom\")"
        elif [ "$code" != 0 ] && grep -ciE -- "timed out" "$log" >/dev/null; then
            caught=$((caught + 1))
            echo "caught: $name by $(basename "$gate" .sh) (exit $code; a program ran past the limit)"
        elif [ "$code" != 0 ]; then
            MISSED+=("$name: $(basename "$gate" .sh) failed (exit $code) but not with \"$symptom\"; see its last lines below")
            tail -5 "$log" | sed 's/^/    /'
        else
            MISSED+=("$name: $(basename "$gate" .sh) passed with the fault in place")
        fi
    done
}

M=compiler/x86_64/src/mach.gr

control opcode-bit $M \
    $'        let op = match m {\n            1 -> 1\n' \
    $'        let op = match m {\n            1 -> 3\n' -- \
    scripts/encodetest_x64.sh "MISMATCH add-rr"

control rex-r $M \
    "    v = v.bit_or(r.shr(3).bit_and(1).shl(2))" \
    "    v = v.bit_or(r.shr(3).bit_and(1).shl(3))" -- \
    scripts/encodetest_x64.sh "MISMATCH add-rr"

control imm8-edge $M \
    "    v >= -128 and v <= 127" "    v >= -128 and v <= 128" -- \
    scripts/encodetest_x64.sh "MISMATCH add-ri"

control sib $M \
    $'    if lo == 4 {\n        b.push(36)' $'    if lo == 12 {\n        b.push(36)' -- \
    scripts/encodetest_x64.sh "MISMATCH load"

control disp-rbp $M \
    "    if disp == 0 and lo != 5 {" "    if disp == 0 {" -- \
    scripts/encodetest_x64.sh "MISMATCH load"

control lock-order $M \
    $'        if sz == 2 {\n            pre.push(102)\n        }\n        if m >= X_XADD8 {\n            pre.push(240)\n        }' \
    $'        if m >= X_XADD8 {\n            pre.push(240)\n        }\n        if sz == 2 {\n            pre.push(102)\n        }' -- \
    scripts/encodetest_x64.sh "MISMATCH xadd16"

control jmp-rel $M \
    "        return [if m == X_JMP8 { 235 } else { 112 + cc }, rel.bit_and(255)]" \
    "        return [if m == X_JMP8 { 235 } else { 112 + cc }, (rel + 1).bit_and(255)]" -- \
    scripts/encodetest_x64.sh "MISMATCH jmp8"

control reloc-kind $M \
    $'    } else if m == X_LEA_RIP {\n        REL_SIGNED' $'    } else if m == X_LEA_RIP {\n        REL_GOT_LOAD' -- \
    scripts/encodetest_x64.sh "RELOC lea-rip"

control coverage compiler/x86_64/src/main.gr \
    $'            out.push(mach.form_name(m))\n        }\n    }\n    out' \
    $'            out.push(mach.form_name(m))\n        }\n    }\n    out.push(mach.form_name(mach.P_RETURN))\n    out' -- \
    scripts/encodetest_x64.sh "with no test cases"

X=compiler/x86_64/src

control x-cc-map $X/rules.gr \
    "    [mach.C_E, mach.C_NE, mach.C_L, mach.C_LE," "    [mach.C_E, mach.C_NE, mach.C_LE, mach.C_LE," -- \
    scripts/encodetest_x64.sh "slt maps to le" - \
    scripts/xs3subset.sh "MISMATCH:" --no-rule-check

control x-fcc-swap $X/rules.gr \
    'rs.push(r("fcmp-swap", ir.M_FCMP, G_FCC_SWAP, cat([tm(mach.X_UCOMISD, S_NONE, S_B, S_A)], fset)))' \
    'rs.push(r("fcmp-swap", ir.M_FCMP, G_FCC_SWAP, cat([tm(mach.X_UCOMISD, S_NONE, S_A, S_B)], fset)))' -- \
    scripts/encodetest_x64.sh "on doubles,"

control x-flags $X/rules.gr \
    "tm(mach.X_BSR, S_RES, S_NONE, S_A), tcc(mach.X_CMOV" \
    "tm(mach.X_BSR, S_RES, S_NONE, S_A), ti(mach.X_NOT, S_T0, 0), tcc(mach.X_CMOV" -- \
    scripts/encodetest_x64.sh "reads flags"

control x-pop-order $X/emit.gr \
    "try ins(p, mach.X_POP, saved[k], -1, -1, -1, 0, -1)" \
    "try ins(p, mach.X_POP, saved[saved.len() - 1 - k], -1, -1, -1, 0, -1)" -- \
    scripts/xs3subset.sh "FAILED:" - \
    "FUZZ_X64_ONLY=^fuzz_(s3|intwidth)$ scripts/fuzz_x64.sh" "FAILED: fuzz_s3"

control x-clobber $X/desc.gr \
    "    t.callee_order = [[mach.RBX, mach.R12," "    t.callee_order = [[mach.RCX, mach.RBX, mach.R12," -- \
    scripts/xs3subset.sh "register allocation check failed"

control x-scratch $X/desc.gr \
    "        [mach.RSI, mach.RDI, mach.R8, mach.R9, mach.RCX, mach.RDX, mach.RAX]," \
    "        [mach.R11, mach.RSI, mach.RDI, mach.R8, mach.R9, mach.RCX, mach.RDX, mach.RAX]," -- \
    scripts/ra_check_x64.sh "register allocation check failed"

control x-reloc-off $X/emit.gr \
    "                    p.rel_in.push(bs.len() - 4)" "                    p.rel_in.push(bs.len() - 3)" -- \
    scripts/objtest_x64.sh "has no BRANCH relocation"

control x-line $X/emit.gr \
    "        row_byte.push(start[p.row_piece[k]])" "        row_byte.push(start[p.row_piece[k]] + 1)" -- \
    scripts/objtest_x64.sh "maps to line"

control x-nondet compiler/s3/src/main.gr \
    '    unit.producer = "grasm compiler/s3"' '    unit.producer = "grasm compiler/s3 at {now_millis()}"' -- \
    scripts/objtest_x64.sh "two compilations differ"

control x-stub compiler/link/src/apply.gr \
    "            let slot = l.o_addr[got_o] + 8 * l.g_got[g]
            let at = l.o_off[stubs_o] + 6 * i" "            let slot = l.o_addr[got_o] + 8 * l.g_got[g] + 8
            let at = l.o_off[stubs_o] + 6 * i" -- \
    scripts/xrtswap.sh "FAILED:"

control x-tlv-lea compiler/link/src/apply.gr \
    "    rd.put8(buf, at - 2, 0x8d)" "    rd.put8(buf, at - 2, 0x8b)" -- \
    scripts/xrtswap.sh "FAILED:"

control x-ctx-switch $X/leaves.gr \
    "    try put(l, mach.X_LOAD, mach.RSP, mach.RSI, -1, CTX_SP)" \
    "    try put(l, mach.X_LOAD, mach.RSP, mach.RSI, -1, CTX_SP + 8)" -- \
    scripts/xrtswap.sh "FAILED:"

L=compiler/link/src

control x-ld-sign $L/image.gr \
    $'fn signs(l: Ld) -> bool {\n    not l.x64' $'fn signs(l: Ld) -> bool {\n    true' -- \
    scripts/linkdiff_x64.sh "otool -l differs"

control x-ld-room $L/layout.gr \
    "    let room = header_size + if l.x64 { 48 } else { 32 }" "    let room = header_size + 32" -- \
    scripts/linkdiff_x64.sh "first difference"

control x-ld-pages $L/image.gr \
    "            let pg = (fx.addr[i] - start) / psz + 1" "            let pg = size / psz" -- \
    scripts/linkdiff_x64.sh "differ"

control x-ld-tail $L/image.gr \
    "            if l.a_tail[a] and not l.dead_strip and l.a_addr" "            if false and l.a_tail[a] and not l.dead_strip and l.a_addr" -- \
    scripts/linkdiff_x64.sh "the whole runtime"

control x-ld-got $L/apply.gr \
    "                    } else if t == objfile.R_X_GOT or (t == objfile.R_X_GOT_LOAD and k == state.T_IMPORT) {" \
    "                    } else if (t == objfile.R_X_GOT or t == objfile.R_X_GOT_LOAD) and k == state.T_IMPORT {" -- \
    scripts/linkdiff_x64.sh "the whole runtime"

control x-ld-dwarf $L/deadstrip.gr \
    "    if l.x64 { 0x04000000 } else { 0x03000000 }" "    0x03000000" -- \
    scripts/linkdiff_x64.sh "the whole runtime"

control x-ld-tlvalign $L/layout.gr \
    "            l.o_p2[o] = tp2" "            l.o_p2[o] = l.o_p2[o]" -- \
    scripts/linkdiff_x64.sh "fixture tlv"

control x-ld-lsda $L/unwind.gr \
    "+ l.eh_lsda_off[e] - (here + lf)" "+ l.eh_lsda_off[e] - (here + lf) + 4" -- \
    scripts/linkdiff_x64.sh "the whole runtime"

control x-ld-strip $L/image.gr \
    "    let le_page = if l.strip { layout.PAGE } else { layout.page(l) }" "    let le_page = layout.page(l)" -- \
    scripts/linkdiff_x64.sh "first difference"

echo
echo "controls: $controls   caught: $caught   missed: ${#MISSED[@]}"
if [ ${#MISSED[@]} -gt 0 ]; then
    echo
    echo "MISSED:"
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
# Thirty faults in thirty-two gate runs, and every one must have been
# caught.
if [ "$controls" -ne 32 ] || [ "$caught" -ne 32 ]; then
    echo "expected 32 controls caught, ran $controls and caught $caught"
    exit 1
fi
exit 0
