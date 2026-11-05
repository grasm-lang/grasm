#!/usr/bin/env bash
# Mutation controls for the Linux gates (docs/self-hosting.md §6.18, stage
# S7): a gate that cannot fail checks nothing.
#
# Each control copies `compiler/` aside, plants one fault (`scripts/
# s3_mutate.py`, which refuses a mutation that matches nothing), builds that
# broken driver, and runs the gate named for it with `S3=` pointing at it,
# stopping at the first program with a problem. The gate must fail, and fail
# for the planted reason: its log must show the symptom that fault produces.
#
# The ELF object (step two), each caught by objtest_elf:
#
#   elf-addend     a call's PLT32 relocation is written with addend 0, not -4,
#                  so it would land four bytes past its function
#   elf-alloc      a function's section lacks SHF_ALLOC, so it would not be
#                  loaded
#   elf-bind       the global functions are written local, `main` among them
#   elf-locals     `.symtab`'s first global is said to be one later than it is
#   elf-lo12       an `add`'s low-twelve-bits relocation is a 64-bit load's,
#                  which scales the offset by eight
#   elf-tprel      the runtime's thread-local getter's high twelve bits of the
#                  offset from the thread pointer are relocated as the low
#                  twelve (the runtime's objects)
#   elf-abbrev     `.debug_info`'s offset into `.debug_abbrev` is left
#                  unrelocated, right in the object and wrong once ld
#                  concatenates the sections of several
#   elf-line       each line sequence starts four bytes into its function
#   elf-stack      no `.note.GNU-stack`, so ld would make the stack executable
#   elf-nondet     the driver writes the clock into every object
#
# The backend and the runtime for Linux (step three):
#
#   elf-pac        a Linux frame returns by `retab`, which is no hint and is
#                  undefined on a processor without pointer authentication
#                  -> objtest_elf (the machine's processor runs it, so no
#                  run would show it)
#   elf-errno      the runtime's `__error` keeps its Darwin name on Linux
#                  -> elfsubset (glibc has no `__error`: no link)
#   elf-tls        the runtime's thread-local getter on arm64 starts from 0
#                  rather than the thread pointer -> elfsubset
#   elf-variadic   `open`'s and `fcntl`'s variadic argument is passed on the
#                  stack, as on Apple's arm64, so `fcntl` reads 0 from a
#                  register: no descriptor is closed on `exec`, and the
#                  processes' conformance test waits past the limit
#                  -> elfsubset. (`open`'s mode is 0 the same way, which no
#                  gate sees: the programs' /tmp is on the Mac's disk, and
#                  OrbStack's mount of it does not hold a file's owner to its
#                  mode.)
#   elf-creat      Linux's runtime takes Darwin's `O_CREAT` -> elfsubset
#   elf-dirent     Linux's runtime reads a directory entry's name where
#                  Darwin's is -> elfrtswap (the Cranelift build with each
#                  runtime)
#   elf-sockopt    Linux's runtime takes Darwin's `SOL_SOCKET` -> elfrtswap
#
# The ELF linker (step four), each caught by linkdiff_elf, against GNU ld's
# bytes:
#
#   link-call26    an arm64 call lands one instruction past its target
#   link-slot      a PLT slot's JUMP_SLOT relocation names the next import
#   link-version   an import's version index is the one before its own, so
#                  it requires the wrong version
#   link-relative  a pointer in arm64 data gets no RELATIVE relocation (its
#                  entry left empty), so it would keep its unrelocated
#                  address (on `examples/ranges.gr`: `hello`'s link has no
#                  such pointer)
#   link-tls       arm64's thread-local offsets are laid out as x86-64's
#                  variant, below the thread pointer
#   link-relro     PT_GNU_RELRO stops a word short of the GOT's end, so the
#                  last slot stays writable
#
# And the glibc floor:
#
#   floor-list     `scripts/glibc_floor.txt` without `hypot`, so a program
#                  calling it requires glibc 2.35 unlisted -> linkdiff_elf
#                  (given the list as ELF_GLIBC_LIST)
#
# The runtime's faults are planted in a copy of `compiler/rt`, which the
# gates are given as `ELF_RT_SRC`; the linker's in a copy of `compiler/link`,
# built and given to the gates as `LINK`. Needs what the gates need
# (scripts/lib/linux.sh); without the machine it does not run (exit 77).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. scripts/lib/linux.sh
linux_require
. scripts/lib/fresh.sh
fresh_or_stop "./target/release/grasm"

WORK=$(linux_workdir)
trap 'rm -rf "$WORK"' EXIT

controls=0
caught=0
declare -a MISSED=()

# control NAME FILE OLD NEW -- GATE SYMPTOM [GATE SYMPTOM] ...
# A gate may start with VAR=value words, set for it alone.
control() {
    local name=$1 file=$2 old=$3 new=$4
    shift 5
    rm -rf "$WORK/src"
    mkdir -p "$WORK/src"
    cp -R compiler "$WORK/src/"
    ln -s "$PWD/packages" "$WORK/src/packages"
    if ! python3 scripts/s3_mutate.py "$WORK/src/$file" "$old" "$new"; then
        MISSED+=("$name: the mutation does not apply")
        return
    fi
    local driver="$WORK/s3-$name"
    local -a linker=()
    case $file in
    compiler/link/*)
        # The linker's fault: the backend is the checkout's, the linker the
        # mutated one.
        driver="$WORK/s3"
        if [ ! -x "$driver" ] && ! "$GRASM" build compiler/s3/src/main.gr --native -o "$driver" >"$WORK/build-s3.log" 2>&1; then
            MISSED+=("$name: compiler/s3 does not build")
            return
        fi
        if ! "$GRASM" build "$WORK/src/compiler/link/src/main.gr" --native -o "$WORK/link-$name" >"$WORK/build-$name.log" 2>&1; then
            MISSED+=("$name: the mutated linker does not build")
            sed 's/^/    /' "$WORK/build-$name.log" | head -5
            return
        fi
        linker=("LINK=$WORK/link-$name")
        ;;
    *)
        if ! "$GRASM" build "$WORK/src/compiler/s3/src/main.gr" --native -o "$driver" >"$WORK/build-$name.log" 2>&1; then
            MISSED+=("$name: the mutated backend does not build")
            sed 's/^/    /' "$WORK/build-$name.log" | head -5
            return
        fi
        ;;
    esac
    while [ $# -ge 2 ]; do
        local gate=$1 symptom=$2
        shift 2
        controls=$((controls + 1))
        local -a cmd
        read -ra cmd <<<"$gate"
        gate=${cmd[${#cmd[@]} - 1]}
        local log="$WORK/$name-$(basename "$gate" .sh).log"
        S3="$driver" ELF_RT_SRC="$WORK/src/compiler/rt" S3_STOP_AT_FIRST=1 env ${linker[@]+"${linker[@]}"} "${cmd[@]}" >"$log" 2>&1
        local code=$?
        if [ "$code" = 77 ]; then
            MISSED+=("$name: $(basename "$gate" .sh) did not run: $(tail -1 "$log")")
        elif [ "$code" != 0 ] && grep -cF -- "$symptom" "$log" >/dev/null; then
            caught=$((caught + 1))
            echo "caught: $name by $(basename "$gate" .sh) (exit $code; \"$symptom\")"
        elif [ "$code" != 0 ]; then
            MISSED+=("$name: $(basename "$gate" .sh) failed (exit $code) but not with \"$symptom\"; see its last lines below")
            tail -5 "$log" | sed 's/^/    /'
        else
            MISSED+=("$name: $(basename "$gate" .sh) passed with the fault in place")
        fi
    done
}

E=compiler/elf/src/main.gr
D=compiler/macho/src/dwarf.gr
OBJ="DIRS=examples scripts/objtest_elf.sh"

control elf-addend $E \
    "        return (R_X86_64_PLT32, -4)" "        return (R_X86_64_PLT32, 0)" -- \
    "ELF_TARGETS=x86_64-linux $OBJ" "R_X86_64_PLT32 +0"

control elf-alloc $E \
    "add_sect(s, tn, SHT_PROGBITS, SHF_ALLOC + SHF_EXECINSTR," "add_sect(s, tn, SHT_PROGBITS, SHF_EXECINSTR," -- \
    "ELF_TARGETS=aarch64-linux $OBJ" "flags 0x4"

control elf-bind $E \
    "        sinfo.push(STB_GLOBAL.shl(4).bit_or(STT_FUNC))" "        sinfo.push(STB_LOCAL.shl(4).bit_or(STT_FUNC))" -- \
    "ELF_TARGETS=aarch64-linux $OBJ" "no global function main"

control elf-locals $E \
    "    s.info[stx - 1] = first_global" "    s.info[stx - 1] = first_global + 1" -- \
    "ELF_TARGETS=x86_64-linux $OBJ" "on the wrong side of"

control elf-lo12 $E \
    $'            return R_AARCH64_ADD_ABS_LO12_NC\n' $'            return R_AARCH64_LDST64_ABS_LO12_NC\n' -- \
    "ELF_TARGETS=aarch64-linux $OBJ" "R_AARCH64_LDST64_ABS_LO12_NC"

control elf-tprel $E \
    $'    if kind == macho.R_TPREL_HI12 {\n        return R_AARCH64_TLSLE_ADD_TPREL_HI12' \
    $'    if kind == macho.R_TPREL_HI12 {\n        return R_AARCH64_TLSLE_ADD_TPREL_LO12_NC' -- \
    "ELF_TARGETS=aarch64-linux $OBJ" "TLSLE_ADD_TPREL_LO12_NC"

control elf-abbrev $D \
    $'    d.o_off.push(info.len())\n    d.o_sect.push(DWS_ABBREV)\n    d.o_add.push(0)\n' "" -- \
    "ELF_TARGETS=aarch64-linux $OBJ" "that is not relocated"

control elf-line $D \
    $'        d.a_fn.push(lt.rel_fn[q])\n        d.a_add.push(0)' $'        d.a_fn.push(lt.rel_fn[q])\n        d.a_add.push(4)' -- \
    "ELF_TARGETS=x86_64-linux $OBJ" "ends at"

control elf-stack $E \
    '    let nx = add_sect(s, ".note.GNU-stack", SHT_PROGBITS, 0, 1, empty())' \
    '    let nx = add_sect(s, ".note.GNU-stck", SHT_PROGBITS, 0, 1, empty())' -- \
    "ELF_TARGETS=aarch64-linux $OBJ" "the stack would be executable"

control elf-nondet compiler/s3/src/main.gr \
    '    unit.producer = "grasm compiler/s3"' '    unit.producer = "grasm compiler/s3 at {now_millis()}"' -- \
    "ELF_TARGETS=aarch64-linux $OBJ" "two compilations differ"

# The backend and the runtime for Linux (step three):
SUB="DIRS=spec/conformance scripts/elfsubset.sh"
SWAP="DIRS=spec/conformance scripts/elfrtswap.sh"

control elf-pac compiler/aarch64/src/emit.gr \
    $'        try word(e, mach.A_AUTIASP, -1, -1, -1, 0, 0)\n        try word(e, mach.A_RET, -1, -1, -1, 0, 0)' \
    $'        try word(e, mach.A_RETAB, -1, -1, -1, 0, 0)' -- \
    "ELF_TARGETS=aarch64-linux $OBJ" "not the hint forms Linux frames use"

control elf-errno compiler/s3/src/main.gr \
    '    if linux and lib and name == "__error" {' '    if false and lib and name == "__error" {' -- \
    "ELF_TARGETS=x86_64-linux ELF_PROGRAMS=hello DIRS=examples scripts/elfsubset.sh" "undefined reference to"

control elf-tls compiler/aarch64/src/leaves.gr \
    "    try put(l, mach.A_MRS_TP, 0, -1, -1, 0)" "    try put(l, mach.A_MOVZ, 0, -1, -1, 0)" -- \
    "ELF_TARGETS=aarch64-linux ELF_PROGRAMS=hello DIRS=examples scripts/elfsubset.sh" "FAILED:"

control elf-variadic compiler/aarch64/src/isel.gr \
    "        fixed = variadic_fixed(vrt[f.imm[i]])" "        fixed = -1" -- \
    "ELF_TARGETS=aarch64-linux ELF_PROGRAMS=L-LIB-PROC $SUB" "s3 124"

control elf-creat compiler/rt/os/linux.gr \
    "pub let O_CREAT = 64" "pub let O_CREAT = 512" -- \
    "ELF_TARGETS=x86_64-linux ELF_PROGRAMS=L-LIB-FS $SUB" "FAILED:"

control elf-dirent compiler/rt/os/linux.gr \
    "pub let DIRENT_NAME = 19" "pub let DIRENT_NAME = 21" -- \
    "ELF_TARGETS=aarch64-linux ELF_PROGRAMS=L-LIB-FS $SWAP" "FAILED:"

control elf-sockopt compiler/rt/os/linux.gr \
    "pub let SOL_SOCKET = 1" "pub let SOL_SOCKET = 65535" -- \
    "ELF_TARGETS=x86_64-linux ELF_PROGRAMS=L-LIB-NET.gr $SWAP" "FAILED:"

# The ELF linker (step four):
L=compiler/link/src/elflink.gr
LD="DIRS=examples ELF_PROGRAMS=hello scripts/linkdiff_elf.sh"

control link-call26 $L \
    $'                        let disp = target - p\n' $'                        let disp = target - p + 4\n' -- \
    "ELF_TARGETS=aarch64-linux $LD" ".text: contents differ"

control link-slot $L \
    "rd.put64(out, e + 8, l.g_dyn[g].shl(32).bit_or(if x64 { X_JUMP_SLOT } else { A_JUMP_SLOT }))" \
    "rd.put64(out, e + 8, (l.g_dyn[g] + 1).shl(32).bit_or(if x64 { X_JUMP_SLOT } else { A_JUMP_SLOT }))" -- \
    "ELF_TARGETS=x86_64-linux $LD" ".rela.plt: contents differ"

control link-version $L \
    "        l.g_vidx[g] = found" "        l.g_vidx[g] = if found > 2 { found - 1 } else { found }" -- \
    "ELF_TARGETS=aarch64-linux $LD" ".gnu.version: contents differ"

control link-relative $L \
    $'                    if t == A_ABS64 {\n                        rd.put64(out, p_off, sv + a)\n                        if alloc {' \
    $'                    if t == A_ABS64 {\n                        rd.put64(out, p_off, sv + a)\n                        if false {' -- \
    "ELF_TARGETS=aarch64-linux DIRS=examples ELF_PROGRAMS=ranges scripts/linkdiff_elf.sh" ".rela.dyn: contents differ"

control link-tls $L \
    "                        let tp = sv + a - (l.tls_addr - align_up(16, first_tls_align(l, relro_secs)))" \
    "                        let tp = sv + a - l.tls_size - l.tls_addr" -- \
    "ELF_TARGETS=aarch64-linux $LD" ".text: contents differ"

control link-relro $L \
    "    l.relro_end = d2" "    l.relro_end = d2 - 8" -- \
    "ELF_TARGETS=x86_64-linux $LD" "GNU_RELRO"

# The glibc floor: a list without `hypot`, and a program that calls it.
grep -v '^hypot' scripts/glibc_floor.txt >"$WORK/floor-list.txt"
controls=$((controls + 1))
ELF_GLIBC_LIST="$WORK/floor-list.txt" ELF_TARGETS=x86_64-linux DIRS=spec/conformance ELF_PROGRAMS=L-LIB-MATH S3_STOP_AT_FIRST=1 \
    scripts/linkdiff_elf.sh >"$WORK/floor-list.log" 2>&1
code=$?
symptom="hypot@GLIBC_2.35 -- above the floor 2.34"
if [ "$code" = 77 ]; then
    MISSED+=("floor-list: linkdiff_elf did not run: $(tail -1 "$WORK/floor-list.log")")
elif [ "$code" != 0 ] && grep -cF -- "$symptom" "$WORK/floor-list.log" >/dev/null; then
    caught=$((caught + 1))
    echo "caught: floor-list by linkdiff_elf (exit $code; \"$symptom\")"
elif [ "$code" != 0 ]; then
    MISSED+=("floor-list: linkdiff_elf failed (exit $code) but not with \"$symptom\"")
    tail -5 "$WORK/floor-list.log" | sed 's/^/    /'
else
    MISSED+=("floor-list: linkdiff_elf passed with hypot unlisted")
fi

echo
echo "controls: $controls   caught: $caught   missed: ${#MISSED[@]}"
if [ ${#MISSED[@]} -gt 0 ]; then
    echo
    echo "MISSED:"
    printf '  %s\n' "${MISSED[@]}"
    exit 1
fi
if [ "$controls" -ne 24 ] || [ "$caught" -ne 24 ]; then
    echo "expected 24 controls caught, ran $controls and caught $caught"
    exit 1
fi
exit 0
