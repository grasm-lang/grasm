#!/usr/bin/env bash
# The Grasm backend's ELF objects, for arm64 and x86-64 Linux, are objects
# the system's tools accept, and the same TIR makes the same bytes
# (docs/self-hosting.md §6.18, stage S7), as `objtest` and `objtest_x64` hold
# the Mach-O ones. For every corpus program in the subset, the object `s3
# --target aarch64-linux` and the one `s3 --target x86_64-linux` write must:
#
#   * be bit-identical when compiled a second time, into another path;
#   * be read by `llvm-readelf` with no warning, as ELF64 relocatable for its
#     machine, defining `main` as a global function;
#   * be laid out as `compiler/elf` says and carry, in each function's
#     section, a relocation of the type each instruction takes -- CALL26 on
#     every `bl`, a page relocation on every `adrp`, the low twelve bits'
#     type on the `add` or the load beside it; PLT32 on every `call`, PC32 on
#     every RIP-relative `leaq`, a GOT relocation on every RIP-relative
#     `movq` -- and none anywhere else; every struct descriptor's addresses,
#     and nothing else in the data, relocated; every DWARF address a
#     function's and every DWARF offset another debug section's
#     (`scripts/elf_obj.py relocs`);
#   * carry a line table `llvm-dwarfdump --verify` finds no error in;
#   * map, by the line table decoded with its relocations, every instruction
#     that loads the address of a string literal written once in the program
#     to the file and line the literal is on (`elf_obj.py lines`), the count
#     held to a floor;
#   * be read by GNU `readelf -a -W` in the Linux machine with nothing on its
#     standard error;
#   * link with the machine's GNU ld (through gcc for arm64; the x86-64 cross
#     ld for x86-64) and the Rust runtime built for its triple, with
#     `--gc-sections`, with nothing on the linker's output.
#
# The Grasm runtime built for each target (`linux_rt_build`: `compiler/rt`
# with `os/linux.gr`, `s3 --runtime --target <t>`) is held to the same,
# but `main`, and to a relocatable link by GNU ld of it alone.
#
# Needs the OrbStack machine (scripts/lib/linux.sh) and the x86-64 cross
# packages (`GRASM_X64_ROOT`); without them it does not run (exit 77). The
# Rust runtimes for the two triples are built as `crosstest` builds its own,
# into `${CARGO_TARGET_DIR:-./target}`; `ELF_RT_A64=` and `ELF_RT_X64=` name
# others. `S3_STOP_AT_FIRST=1` stops at the first program with a problem
# (the controls, `scripts/elfcontrols.sh`); `ELF_TARGETS=` limits the targets
# (`aarch64-linux`, `x86_64-linux`).
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. scripts/lib/linux.sh
linux_require
. scripts/lib/fresh.sh
fresh_or_stop "./target/release/grasm"
. scripts/lib/corpus.sh
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require

WORK=$(linux_workdir)
trap 'rm -rf "$WORK"' EXIT
linux_x64_root
. scripts/lib/s3.sh
s3_driver
READELF=${LLVM_READELF:-/opt/homebrew/opt/llvm@21/bin/llvm-readelf}
DWARFDUMP=${LLVM_DWARFDUMP:-/opt/homebrew/opt/llvm@21/bin/llvm-dwarfdump}
for tool in "$READELF" "$DWARFDUMP"; do
    [ -x "$tool" ] || { echo "error: $tool is missing (Homebrew's llvm@21; LLVM_READELF= and LLVM_DWARFDUMP= name others)"; exit 1; }
done
TARGETS=${ELF_TARGETS:-aarch64-linux x86_64-linux}
CARGO_OUT=${CARGO_TARGET_DIR:-./target}
for t in $TARGETS; do
    case $t in
    aarch64-linux) triple=aarch64-unknown-linux-gnu; var=ELF_RT_A64 ;;
    x86_64-linux) triple=x86_64-unknown-linux-gnu; var=ELF_RT_X64 ;;
    *) echo "error: ELF_TARGETS names $t"; exit 1 ;;
    esac
    if [ -z "${!var:-}" ]; then
        if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --target "$triple"; then
            echo "error: the runtime for $triple does not build (rustup target add $triple)"
            exit 1
        fi
        printf -v "$var" '%s' "$(cd "$CARGO_OUT/$triple/release" && pwd -P)/libgrasm_rt_abi.a"
    fi
    [ -f "${!var}" ] || { echo "error: ${!var} is missing"; exit 1; }
done
s3_collect
STOP=${S3_STOP_AT_FIRST:-}
stopped=0
problems=0
declare -a FAILED=()
declare -a CHECKED=() IDENTICAL=() LINES=() REPEATED=() RELOCS=() LINKED=() RT_RELOCS=()
# shellcheck disable=SC2206
TG=($TARGETS)
mkdir -p "$WORK/b/deeper"
: >"$WORK/batch.txt"

for i in "${!TG[@]}"; do
    t=${TG[$i]}
    LINKED[$i]=0
    CHECKED[$i]=0
    IDENTICAL[$i]=0
    LINES[$i]=0
    REPEATED[$i]=0
    RELOCS[$i]=0
    mkdir -p "$WORK/obj/$t"
    machine=$([ "$t" = x86_64-linux ] && echo "X86-64" || echo "AArch64")
    for k in "${!S3_SRC[@]}"; do
        if [ -n "$STOP" ] && [ "$problems" -gt 0 ]; then
            stopped=1
            break 2
        fi
        [ "${S3_RULE[$k]}" = "in" ] || continue
        src=${S3_SRC[$k]}
        tir=${S3_TIR[$k]}
        CHECKED[$i]=$((CHECKED[$i] + 1))
        a="$WORK/obj/$t/$k.o"
        b="$WORK/b/deeper/q.o"
        rm -f "$a" "$b"
        if ! "$S3" "$tir" -o "$a" --target "$t" >"$WORK/s3.log" 2>&1 || ! "$S3" "$tir" -o "$b" --target "$t" >>"$WORK/s3.log" 2>&1; then
            problems=$((problems + 1))
            FAILED+=("$t $src — the backend does not compile it: $(head -1 "$WORK/s3.log")")
            continue
        fi
        bad=""
        if cmp -s "$a" "$b"; then
            IDENTICAL[$i]=$((IDENTICAL[$i] + 1))
        else
            bad="$bad; two compilations differ"
        fi
        if "$READELF" -h -S -s -r -W "$a" >"$WORK/re.txt" 2>"$WORK/re.err"; then
            [ -s "$WORK/re.err" ] && bad="$bad; llvm-readelf warns: $(head -1 "$WORK/re.err")"
            grep -c "Type:.*REL (Relocatable file)" "$WORK/re.txt" >/dev/null || bad="$bad; llvm-readelf: not a relocatable file"
            grep -c "Machine:.*$machine" "$WORK/re.txt" >/dev/null || bad="$bad; llvm-readelf: not $machine"
            grep -cE ' FUNC +GLOBAL +DEFAULT +[0-9]+ main$' "$WORK/re.txt" >/dev/null || bad="$bad; llvm-readelf: no global function main"
        else
            bad="$bad; llvm-readelf refuses it: $(head -1 "$WORK/re.err")"
        fi
        if python3 scripts/elf_obj.py relocs "$a" >"$WORK/relocs.txt" 2>&1; then
            n=$(sed -n 's/.*   relocations: \([0-9]*\) .*/\1/p' "$WORK/relocs.txt" | tail -1)
            RELOCS[$i]=$((RELOCS[$i] + ${n:-0}))
        else
            bad="$bad; relocations: $(head -2 "$WORK/relocs.txt" | tr '\n' ' ')"
        fi
        if ! "$DWARFDUMP" --verify "$a" >"$WORK/verify.txt" 2>&1 || ! grep -cx "No errors." "$WORK/verify.txt" >/dev/null; then
            bad="$bad; llvm-dwarfdump --verify: $(grep -iE 'error|warning' "$WORK/verify.txt" | head -2 | tr '\n' ' ')"
        fi
        if python3 scripts/elf_obj.py lines "$a" "$tir" >"$WORK/lines.txt" 2>&1; then
            n=$(sed -n 's/.*literal loads checked: \([0-9]*\).*/\1/p' "$WORK/lines.txt" | tail -1)
            LINES[$i]=$((LINES[$i] + ${n:-0}))
            n=$(sed -n 's/.*of literals loaded more than once: \([0-9]*\).*/\1/p' "$WORK/lines.txt" | tail -1)
            REPEATED[$i]=$((REPEATED[$i] + ${n:-0}))
        else
            bad="$bad; line table: $(head -2 "$WORK/lines.txt" | tr '\n' ' ')"
        fi
        if [ -n "$bad" ]; then
            problems=$((problems + 1))
            FAILED+=("$t $src${bad}")
        fi
        printf '%s\t%s\t%s\t%s\n' "$t" "$k" "$a" "$i" >>"$WORK/batch.txt"
    done
done

# The runtime, built for each target as the Linux gates build it
# (`linux_rt_build`), twice, held to the same checks but `main`'s, and to
# GNU ld's relocatable link of it alone in the machine.
declare -a RT_OK=()
for i in "${!TG[@]}"; do
    [ "$stopped" = 1 ] && break
    t=${TG[$i]}
    RT_OK[$i]=0
    r="$WORK/rt-$t.o"
    bad=""
    if ! linux_rt_build "$t" "$r" || ! mv "$r" "$r.first" || ! linux_rt_build "$t" "$r"; then
        problems=$((problems + 1))
        FAILED+=("$t the runtime — does not build: $(head -2 "$r.log" | tr '\n' ' ')")
        continue
    fi
    cmp -s "$r" "$r.first" || bad="$bad; two builds differ"
    "$READELF" -h -S -s -r -W "$r" >"$WORK/re.txt" 2>"$WORK/re.err" || bad="$bad; llvm-readelf refuses it"
    [ -s "$WORK/re.err" ] && bad="$bad; llvm-readelf warns: $(head -1 "$WORK/re.err")"
    if python3 scripts/elf_obj.py relocs "$r" >"$WORK/relocs.txt" 2>&1; then
        n=$(sed -n 's/.*   relocations: \([0-9]*\) .*/\1/p' "$WORK/relocs.txt" | tail -1)
        RT_RELOCS[$i]=${n:-0}
    else
        bad="$bad; relocations: $(head -2 "$WORK/relocs.txt" | tr '\n' ' ')"
    fi
    if ! "$DWARFDUMP" --verify "$r" >"$WORK/verify.txt" 2>&1 || ! grep -cx "No errors." "$WORK/verify.txt" >/dev/null; then
        bad="$bad; llvm-dwarfdump --verify: $(grep -iE 'error|warning' "$WORK/verify.txt" | head -2 | tr '\n' ' ')"
    fi
    if [ -n "$bad" ]; then
        problems=$((problems + 1))
        FAILED+=("$t the runtime$bad")
    else
        RT_OK[$i]=1
    fi
    printf '%s\t%s\t%s\t%s\n' "$t" "rt" "$r" "$i" >>"$WORK/batch.txt"
done

# The machine's half, in one start of it: GNU readelf, then a link per
# object, four at a time, each writing what the tools said beside the object
# (the runtime's alone, relocatable).
cat >"$WORK/machine.sh" <<EOF
#!/bin/bash
one() {
    t=\$1; o=\$3
    readelf -a -W "\$o" >/dev/null 2>"\$o.readelf"
    if [ "\$k" = rt ]; then
        if [ "\$t" = aarch64-linux ]; then
            ld -r "\$o" -o "\$o.r" >"\$o.link" 2>&1 || echo "exit \$?" >>"\$o.link"
        else
            "$X64_LD" -r "\$o" -o "\$o.r" >"\$o.link" 2>&1 || echo "exit \$?" >>"\$o.link"
        fi
        rm -f "\$o.r"
    elif [ "\$t" = aarch64-linux ]; then
        gcc "\$o" "$ELF_RT_A64" -lm -o "\$o.exe" -Wl,--gc-sections >"\$o.link" 2>&1 || echo "exit \$?" >>"\$o.link"
    else
        "$X64_LD" -pie -z now -dynamic-linker /lib64/ld-linux-x86-64.so.2 $X64_ROOT$X64_LIB/Scrt1.o $X64_ROOT$X64_LIB/crti.o "\$o" "${ELF_RT_X64:-}" -L=$X64_LIB -lm -lc $X64_ROOT$X64_LIB/libgcc_s.so.1 $X64_ROOT$X64_LIB/crtn.o --gc-sections -o "\$o.exe" >"\$o.link" 2>&1 || echo "exit \$?" >>"\$o.link"
    fi
    rm -f "\$o.exe"
}
while IFS="\$(printf '\t')" read -r t k o i; do
    one "\$t" "\$k" "\$o" &
    while [ "\$(jobs -p | wc -l)" -ge 4 ]; do sleep 0.05; done
done <"$WORK/batch.txt"
wait
EOF
chmod +x "$WORK/machine.sh"
if [ "$stopped" = 0 ]; then
    linux_run "$WORK/machine.sh"
    while IFS=$'\t' read -r t k o i; do
        bad=""
        [ -s "$o.readelf" ] && bad="$bad; GNU readelf says: $(head -1 "$o.readelf")"
        if [ ! -f "$o.link" ]; then
            bad="$bad; the machine did not link it"
        elif [ -s "$o.link" ]; then
            bad="$bad; the linker said: $(head -2 "$o.link" | tr '\n' ' ')"
        elif [ "$k" != rt ]; then
            LINKED[$i]=$((LINKED[$i] + 1))
        fi
        if [ -n "$bad" ]; then
            problems=$((problems + 1))
            if [ "$k" = rt ]; then
                RT_OK[$i]=0
                FAILED+=("$t the runtime${bad}")
            else
                FAILED+=("$t ${S3_SRC[$k]}${bad}")
            fi
        fi
    done <"$WORK/batch.txt"
fi

for i in "${!TG[@]}"; do
    t=${TG[$i]}
    echo "$t: objects: ${CHECKED[$i]}   reproducible: ${IDENTICAL[$i]}   linked by ld: ${LINKED[$i]}   relocations: ${RELOCS[$i]}   literal loads on their lines: ${LINES[$i]} (of literals loaded more than once: ${REPEATED[$i]})"
    echo "$t: the runtime: built twice the same, read, relocated and linked (ld -r) without a problem: ${RT_OK[$i]:-0} of 1   relocations: ${RT_RELOCS[$i]:-0}"
done
echo "with problems: $problems   ($("$READELF" --version | grep -m1 -i version | sed 's/^ *//'); $(linux_run sh -c 'readelf --version | head -1'))"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -60
fi
if [ "$stopped" = 1 ]; then
    echo
    echo "stopped at the first failure (S3_STOP_AT_FIRST); the counts above are partial"
    exit 1
fi
[ "$problems" -eq 0 ] || exit 1
for i in "${!TG[@]}"; do
    t=${TG[$i]}
    [ "${IDENTICAL[$i]}" -eq "${CHECKED[$i]}" ] || exit 1
    [ "${LINKED[$i]}" -eq "${CHECKED[$i]}" ] || { echo "$t: ${LINKED[$i]} of ${CHECKED[$i]} linked"; exit 1; }
    if [ -z "${DIRS:-}" ] && [ "${CHECKED[$i]}" -lt "$S3_SUBSET_FLOOR" ]; then
        echo "$t: only ${CHECKED[$i]} objects were checked; the subset's floor is $S3_SUBSET_FLOOR"
        exit 1
    fi
    [ "${LINES[$i]}" -gt 0 ] || { echo "$t: no instruction was looked up in a line table"; exit 1; }
    [ "${RT_OK[$i]:-0}" = 1 ] || { echo "$t: the runtime was not checked"; exit 1; }
done
# The line tables' samples, held to floors as `objtest`'s and `objtest_x64`'s
# are: arm64 looks up both instructions that load a literal's address,
# x86-64 its one `leaq`. Far fewer would be a check that stopped finding its
# sample.
if [ -z "${DIRS:-}" ]; then
    for i in "${!TG[@]}"; do
        t=${TG[$i]}
        floor=$([ "$t" = x86_64-linux ] && echo 60000 || echo 120000)
        if [ "${LINES[$i]}" -lt "$floor" ]; then
            echo "$t: only ${LINES[$i]} literal loads were looked up in the line tables; the floor is $floor"
            exit 1
        fi
    done
fi
exit 0
