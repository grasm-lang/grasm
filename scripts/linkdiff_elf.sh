#!/usr/bin/env bash
# `linkdiff` for Linux (docs/self-hosting.md §6.18, stage S7): the Grasm
# linker's ELF executables against GNU ld's, on the same inputs.
#
# For every fourth program of the subset (`scripts/s3_subset.py`), for each
# of `aarch64-linux` and `x86_64-linux`, two links: the Grasm backend's
# object (`s3 --target <t>`) and the Cranelift build's (`grasm build
# --native --target <triple>`), each with the Grasm runtime built for Linux
# (`linux_rt_build`) and `compiler/link`'s `_start`, glibc's `libm.so.6`
# and `libc.so.6`:
#
#   * `compiler/link -sysroot <root> <object> -interpose <runtime>
#     -dead_strip -strip` on the Mac, the C library read from the machine's
#     own root (arm64, through OrbStack's mount) or from `GRASM_X64_ROOT`;
#   * GNU ld in the machine, `-pie -z now -z relro --hash-style=gnu
#     --gc-sections --build-id=none -s`, the same files in the same order,
#     the runtime's names bound by `--defsym` (the x86-64 cross ld for
#     x86-64);
#
# and the two executables must be the same bytes. A difference is named
# part by part (`scripts/elfcmp.py`: the headers, the program headers, each
# section header and each section's bytes). Each link is made twice, and
# must be the same bytes both times.
#
# The glibc floor: the newest GLIBC_ version each of the linker's
# executables requires (`.gnu.version_r`) is the oldest glibc that runs it.
# It must be 2.34 (`__libc_start_main@@GLIBC_2.34`), but for an import of a
# function `scripts/glibc_floor.txt` lists with a newer default version
# (`linux_glibc_floor`); the gate prints the newest over its links and each
# listed function's count.
#
# What the linker does not link yet must be refused, never written wrong:
# the Rust runtime's archive (unwind tables, section groups) is linked once
# per target and must be refused with the reason; a link without `-strip`
# must be refused (no symbol table or debug sections yet).
#
# Needs the OrbStack machine and `GRASM_X64_ROOT` (scripts/lib/linux.sh);
# without them it does not run (exit 77). Counts are derived and asserted.
# `LINK=` and `S3=` test those binaries; `ELF_TARGETS=`, `ELF_PROGRAMS=`
# (every matching program, not every fourth) and `ELF_RT_A64=`/`ELF_RT_X64=`
# as for `scripts/elfsubset.sh`; `S3_STOP_AT_FIRST=1` stops at the first
# difference (the controls, `scripts/elfcontrols.sh`).
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
linux_a64_root
. scripts/lib/s3.sh
s3_driver
. scripts/lib/link.sh
link_driver
TARGETS=${ELF_TARGETS:-aarch64-linux x86_64-linux}
# shellcheck disable=SC2206
TG=($TARGETS)
CARGO_OUT=${CARGO_TARGET_DIR:-./target}
declare -a TRIPLE=() ROOT=() RUST_RT=() GRASM_RT=() START=()
for i in "${!TG[@]}"; do
    t=${TG[$i]}
    case $t in
    aarch64-linux) triple=aarch64-unknown-linux-gnu; var=ELF_RT_A64; arch=aarch64; ROOT[$i]=$A64_ROOT ;;
    x86_64-linux) triple=x86_64-unknown-linux-gnu; var=ELF_RT_X64; arch=x86_64; ROOT[$i]=$X64_ROOT ;;
    *) echo "error: ELF_TARGETS names $t"; exit 1 ;;
    esac
    TRIPLE[$i]=$triple
    if [ -z "${!var:-}" ]; then
        if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --target "$triple"; then
            echo "error: the runtime for $triple does not build (rustup target add $triple)"
            exit 1
        fi
        RUST_RT[$i]="$(cd "$CARGO_OUT/$triple/release" && pwd -P)/libgrasm_rt_abi.a"
    else
        RUST_RT[$i]=${!var}
    fi
    [ -f "${RUST_RT[$i]}" ] || { echo "error: ${RUST_RT[$i]} is missing"; exit 1; }
    GRASM_RT[$i]="$WORK/rt-$t.o"
    if ! linux_rt_build "$t" "${GRASM_RT[$i]}"; then
        echo "error: the Grasm runtime for $t does not build:"
        sed 's/^/  /' "${GRASM_RT[$i]}.log" | head -20
        exit 1
    fi
    if ! linux_defsyms "${GRASM_RT[$i]}" "$WORK/defsyms-$t.txt"; then
        echo "error: the Grasm runtime for $t defines no runtime name"
        exit 1
    fi
    START[$i]="$WORK/start-$t.o"
    if ! "$LINK" -start-object "$arch" -o "${START[$i]}" >"$WORK/start.log" 2>&1; then
        echo "error: compiler/link does not write _start for $arch: $(head -1 "$WORK/start.log")"
        exit 1
    fi
done

s3_collect
linux_filter_programs
STOP=${S3_STOP_AT_FIRST:-}
stopped=0
declare -a CHOSEN=() LINKED=() SAME=() DIFFER=() FLOOR_MAX=() REFUSED=() TWICE=()
declare -a FAILED=()
for i in "${!TG[@]}"; do
    CHOSEN[$i]=0; LINKED[$i]=0; SAME[$i]=0; DIFFER[$i]=0; FLOOR_MAX[$i]=none; REFUSED[$i]=0; TWICE[$i]=0
done
: >"$WORK/batch.txt"

# The Mac's half: compile, and link with compiler/link.
in_n=0
for k in "${!S3_SRC[@]}"; do
    [ "${S3_RULE[$k]}" = in ] || continue
    in_n=$((in_n + 1))
    if [ -z "${ELF_PROGRAMS:-}" ] && [ $((in_n % 4)) -ne 1 ]; then
        continue
    fi
    src=${S3_SRC[$k]}
    for i in "${!TG[@]}"; do
        t=${TG[$i]}
        d="$WORK/p/$t/$k"
        mkdir -p "$d"
        CHOSEN[$i]=$((CHOSEN[$i] + 1))
        if ! "$S3" "${S3_TIR[$k]}" -o "$d/s.o" --target "$t" >"$d/s3.log" 2>&1; then
            FAILED+=("$t $src — the Grasm backend does not compile it: $(head -1 "$d/s3.log")")
            continue
        fi
        if ! "$GRASM" build "$src" --native --target "${TRIPLE[$i]}" -o "$d/c.o" >"$d/c.log" 2>&1 || [ ! -f "$d/c.o" ]; then
            FAILED+=("$t $src — the Cranelift build for ${TRIPLE[$i]} fails: $(head -1 "$d/c.log")")
            continue
        fi
        for w in s c; do
            if ! nice -n 10 "$LINK" -sysroot "${ROOT[$i]}" "$d/$w.o" -interpose "${GRASM_RT[$i]}" -dead_strip -strip \
                -o "$d/$w.g" >"$d/$w.glog" 2>&1; then
                FAILED+=("$t $src ($w) — compiler/link does not link it: $(head -2 "$d/$w.glog" | tr '\n' ' ')")
                continue
            fi
            LINKED[$i]=$((LINKED[$i] + 1))
            if nice -n 10 "$LINK" -sysroot "${ROOT[$i]}" "$d/$w.o" -interpose "${GRASM_RT[$i]}" -dead_strip -strip \
                -o "$d/$w.g2" >/dev/null 2>&1 && cmp -s "$d/$w.g" "$d/$w.g2"; then
                TWICE[$i]=$((TWICE[$i] + 1))
            else
                FAILED+=("$t $src ($w) — linked twice, two executables")
            fi
            rm -f "$d/$w.g2"
            if ! linux_glibc_floor "$d/$w.g" "$WORK/listed-$t.txt" >"$d/$w.floor"; then
                FAILED+=("$t $src ($w) — $(cat "$d/$w.floor")")
            fi
            v=$LINUX_GLIBC_NEWEST
            if [ "$v" != none ] && { [ "${FLOOR_MAX[$i]}" = none ] || linux_version_newer "$v" "${FLOOR_MAX[$i]}"; }; then
                FLOOR_MAX[$i]=$v
            fi
            printf '%s\t%s\t%s\t%s\n' "$i" "$k" "$d" "$w" >>"$WORK/batch.txt"
        done
    done
    if [ -n "$STOP" ] && [ "${#FAILED[@]}" -gt 0 ]; then
        stopped=1
        break
    fi
done

# What is not linked yet is refused: the Rust runtime's archive, and a link
# without -strip.
for i in "${!TG[@]}"; do
    t=${TG[$i]}
    obj=$(awk -F'\t' -v i="$i" '$1 == i && $4 == "c" { print $3 "/c.o"; exit }' "$WORK/batch.txt")
    [ -n "$obj" ] || continue
    "$LINK" -sysroot "${ROOT[$i]}" "$obj" "${RUST_RT[$i]}" -dead_strip -strip -o "$WORK/rust-$t.x" >"$WORK/rust-$t.log" 2>&1
    code=$?
    if [ "$code" = 1 ] && grep -q 'unwind tables (.eh_frame) are not linked yet' "$WORK/rust-$t.log" && [ ! -e "$WORK/rust-$t.x" ]; then
        REFUSED[$i]=$((REFUSED[$i] + 1))
    else
        FAILED+=("$t — the Rust runtime's archive was not refused for its unwind tables (exit $code: $(head -1 "$WORK/rust-$t.log"))")
    fi
    "$LINK" -sysroot "${ROOT[$i]}" "$obj" -interpose "${GRASM_RT[$i]}" -dead_strip -o "$WORK/nostrip-$t.x" >"$WORK/nostrip-$t.log" 2>&1
    code=$?
    if [ "$code" = 1 ] && grep -q 'written stripped only' "$WORK/nostrip-$t.log" && [ ! -e "$WORK/nostrip-$t.x" ]; then
        REFUSED[$i]=$((REFUSED[$i] + 1))
    else
        FAILED+=("$t — a link without -strip was not refused (exit $code: $(head -1 "$WORK/nostrip-$t.log"))")
    fi
done

# The machine's half, in one start of it: GNU ld's links.
{
    echo '#!/bin/bash'
    for i in "${!TG[@]}"; do
        echo "TG[$i]=${TG[$i]}; GRT[$i]='${GRASM_RT[$i]}'; ST[$i]='${START[$i]}'; DS[$i]='$WORK/defsyms-${TG[$i]}.txt'"
    done
    cat <<EOF
X64_LD='$X64_LD'; XR='$X64_ROOT'; XL='$X64_LIB'
FLAGS="-pie -z now -z relro --hash-style=gnu --gc-sections --build-id=none -s"
while IFS="\$(printf '\t')" read -r i k d w; do
    if [ "\${TG[\$i]}" = aarch64-linux ]; then
        nice -n 10 ld \$FLAGS -dynamic-linker /lib/ld-linux-aarch64.so.1 "\${ST[\$i]}" "\$d/\$w.o" "\${GRT[\$i]}" /lib/aarch64-linux-gnu/libm.so.6 /lib/aarch64-linux-gnu/libc.so.6 @"\${DS[\$i]}" -o "\$d/\$w.ld" >"\$d/\$w.ldlog" 2>&1 || echo "exit \$?" >>"\$d/\$w.ldlog"
    else
        nice -n 10 "\$X64_LD" \$FLAGS -dynamic-linker /lib64/ld-linux-x86-64.so.2 -rpath-link "\$XR\$XL" "\${ST[\$i]}" "\$d/\$w.o" "\${GRT[\$i]}" "\$XR\$XL/libm.so.6" "\$XR\$XL/libc.so.6" @"\${DS[\$i]}" -o "\$d/\$w.ld" >"\$d/\$w.ldlog" 2>&1 || echo "exit \$?" >>"\$d/\$w.ldlog"
    fi
done <"$WORK/batch.txt"
EOF
} >"$WORK/machine.sh"
chmod +x "$WORK/machine.sh"
if [ "$stopped" = 0 ] && [ -s "$WORK/batch.txt" ]; then
    linux_run "$WORK/machine.sh"
fi

# The bytes.
while IFS=$'\t' read -r i k d w; do
    [ "$stopped" = 1 ] && break
    t=${TG[$i]}
    src=${S3_SRC[$k]}
    if [ -s "$d/$w.ldlog" ] || [ ! -f "$d/$w.ld" ]; then
        FAILED+=("$t $src ($w) — GNU ld does not link it: $(head -2 "$d/$w.ldlog" | tr '\n' ' ')")
        continue
    fi
    if cmp -s "$d/$w.g" "$d/$w.ld"; then
        SAME[$i]=$((SAME[$i] + 1))
    else
        DIFFER[$i]=$((DIFFER[$i] + 1))
        FAILED+=("$t $src ($w) — not GNU ld's bytes")
        echo "DIFFERENT: $t $src ($([ "$w" = s ] && echo "the Grasm backend's object" || echo "the Cranelift object"))"
        python3 scripts/elfcmp.py "$d/$w.g" "$d/$w.ld" | head -12 | sed 's/^/  /'
        if [ -n "$STOP" ]; then
            stopped=1
        fi
    fi
done <"$WORK/batch.txt"

echo
for i in "${!TG[@]}"; do
    echo "${TG[$i]}: programs: ${CHOSEN[$i]} ($([ -z "${ELF_PROGRAMS:-}" ] && echo "every fourth of the subset" || echo "those ELF_PROGRAMS names"))   linked by compiler/link: ${LINKED[$i]}   twice the same: ${TWICE[$i]}   GNU ld's bytes: ${SAME[$i]}   different: ${DIFFER[$i]}   newest glibc required: ${FLOOR_MAX[$i]} (floor $LINUX_GLIBC_FLOOR; above it, as listed: $(linux_glibc_listed "$WORK/listed-${TG[$i]}.txt"))   refused as not linked yet: ${REFUSED[$i]} of 2"
done
echo "against $(linux_run sh -c 'ld --version | head -1') (arm64) and $(linux_run sh -c "'$X64_LD' --version | head -1") (x86-64)"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -80
fi
if [ "$stopped" = 1 ]; then
    echo
    echo "stopped at the first failure (S3_STOP_AT_FIRST); the counts above are partial"
    exit 1
fi
[ ${#FAILED[@]} -eq 0 ] || exit 1
for i in "${!TG[@]}"; do
    if [ "${LINKED[$i]}" -ne $((2 * CHOSEN[$i])) ] || [ "${SAME[$i]}" -ne "${LINKED[$i]}" ] || [ "${TWICE[$i]}" -ne "${LINKED[$i]}" ]; then
        echo "${TG[$i]}: ${SAME[$i]} of ${LINKED[$i]} links compared equal, of $((2 * CHOSEN[$i])) wanted"
        exit 1
    fi
    if [ "${REFUSED[$i]}" -ne 2 ]; then
        echo "${TG[$i]}: ${REFUSED[$i]} of 2 refusals checked"
        exit 1
    fi
    if [ -z "${ELF_PROGRAMS:-}" ] && [ "${CHOSEN[$i]}" -lt $((S3_SUBSET_FLOOR / 4)) ]; then
        echo "${TG[$i]}: only ${CHOSEN[$i]} programs linked; every fourth of the subset's floor is $((S3_SUBSET_FLOOR / 4))"
        exit 1
    fi
    [ "${SAME[$i]}" -gt 0 ] || { echo "${TG[$i]}: nothing was compared"; exit 1; }
done
exit 0
