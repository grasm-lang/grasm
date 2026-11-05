#!/usr/bin/env bash
# `rtswap` for Linux (docs/self-hosting.md §6.18, stage S7): every corpus
# program, linked with the runtime written in Grasm alone, built for arm64
# or x86-64 Linux, must print and exit exactly as it does with the Rust
# runtime built for the same triple, in the OrbStack machine.
#
# The Grasm runtime for each target is `compiler/rt` with `os/linux.gr` in
# `src/os.gr`'s place, compiled by `s3 --runtime --target <t>`, with its
# poisoned build (`scripts/lib/linux.sh`, `linux_rt_build`), and it must
# export exactly the names `compiler/rt/families.txt` lists and call no
# Rust runtime name, as `rtswap` requires on macOS. Then, for each corpus
# program with a `main` and each target:
#
#   * the Cranelift build's object (`grasm build --native --target
#     <triple>`), linked with the Rust runtime by the machine's toolchain
#     (gcc; the x86-64 cross ld and glibc of `GRASM_X64_ROOT`) and again with
#     the Grasm runtime alone, by `compiler/link` on the Mac (`-sysroot`, the
#     machine's own root or `GRASM_X64_ROOT`; `-interpose`, which binds each
#     of the program's runtime names to the Grasm runtime's; `-dead_strip
#     -strip`);
#   * the same two links from the Rust runtime built with `grasm_rt/poison`
#     and the Grasm runtime's poisoned build, which must agree and report no
#     use after free;
#   * for a program in the Grasm backend's subset, its object (`s3 --target
#     <t>`), linked with each runtime the same two ways.
#
# Each pair runs with stdin closed, x86-64 ones through the cross glibc's
# dynamic loader, each run with a /tmp of its own (`scripts/lib/isolate.py`),
# and stdout, the exit status and stderr must be identical.
# Every runtime name a program's objects reference must be one the Grasm
# runtime defines (counted, distinct and per program). The counts are
# derived and asserted.
#
# Needs the OrbStack machine and `GRASM_X64_ROOT`; without them it does not
# run (exit 77). `S3=`, `LINK=` and `DIRS=` name what to test instead of
# building it; `ELF_TARGETS=` limits the targets, `ELF_PROGRAMS=<regex>` the
# programs (no floor then); `ELF_RT_SRC=` names a copy of `compiler/rt`;
# `ELF_RT_A64=`,
# `ELF_RT_X64=`, `ELF_RTP_A64=` and `ELF_RTP_X64=` name the Rust runtimes
# and their poisoned builds, otherwise built into `${CARGO_TARGET_DIR:-
# ./target}` (the poisoned ones under `elf-poison/` there).
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
linux_isolate_check
. scripts/lib/s3.sh
s3_driver
. scripts/lib/link.sh
link_driver
NM=${LLVM_NM:-/opt/homebrew/opt/llvm@21/bin/llvm-nm}
TARGETS=${ELF_TARGETS:-aarch64-linux x86_64-linux}
# shellcheck disable=SC2206
TG=($TARGETS)
CARGO_OUT=${CARGO_TARGET_DIR:-./target}
moved=$(awk '!/^#/ && NF >= 2 { print $2 }' compiler/rt/families.txt | LC_ALL=C sort -u)
declare -a TRIPLE=() ROOT=() RRT=() RRTP=() GRT=() GRTP=()
for i in "${!TG[@]}"; do
    t=${TG[$i]}
    case $t in
    aarch64-linux) triple=aarch64-unknown-linux-gnu; v=A64; ROOT[$i]=$A64_ROOT ;;
    x86_64-linux) triple=x86_64-unknown-linux-gnu; v=X64; ROOT[$i]=$X64_ROOT ;;
    *) echo "error: ELF_TARGETS names $t"; exit 1 ;;
    esac
    TRIPLE[$i]=$triple
    var=ELF_RT_$v
    if [ -n "${!var:-}" ]; then
        RRT[$i]=${!var}
    else
        if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --target "$triple"; then
            echo "error: the runtime for $triple does not build (rustup target add $triple)"
            exit 1
        fi
        RRT[$i]="$(cd "$CARGO_OUT/$triple/release" && pwd -P)/libgrasm_rt_abi.a"
    fi
    var=ELF_RTP_$v
    if [ -n "${!var:-}" ]; then
        RRTP[$i]=${!var}
    else
        if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --features grasm_rt/poison --target "$triple" --target-dir "$CARGO_OUT/elf-poison"; then
            echo "error: the poisoned runtime for $triple does not build"
            exit 1
        fi
        RRTP[$i]="$(cd "$CARGO_OUT/elf-poison/$triple/release" && pwd -P)/libgrasm_rt_abi.a"
    fi
    for f in "${RRT[$i]}" "${RRTP[$i]}"; do
        [ -f "$f" ] || { echo "error: $f is missing"; exit 1; }
    done
    GRT[$i]="$WORK/rt-$t.o"
    GRTP[$i]="$WORK/rt-$t.poison.o"
    for f in "${GRT[$i]}:" "${GRTP[$i]}:poison"; do
        if ! linux_rt_build "$t" "${f%%:*}" "${f#*:}"; then
            echo "error: the Grasm runtime for $t (${f#*:}) does not build:"
            sed 's/^/  /' "${f%%:*}.log" | head -20
            exit 1
        fi
    done
    # Its names: exactly the moved ones, and no Rust runtime name called.
    exported=$("$NM" -g --defined-only "${GRT[$i]}" | awk '$3 ~ /^grt_/ { sub(/^grt_/, "", $3); print $3 }' | LC_ALL=C sort -u)
    called=$("$NM" -u "${GRT[$i]}" | awk '$NF ~ /^grasm_/ { print $NF }' | tr '\n' ' ')
    if [ "$exported" != "$moved" ]; then
        echo "error: the Grasm runtime for $t does not export exactly the names families.txt lists:"
        diff <(echo "$moved") <(echo "$exported") | head -10
        exit 1
    fi
    if [ -n "$called" ]; then
        echo "error: the Grasm runtime for $t calls the Rust runtime's $called"
        exit 1
    fi
    linux_defsyms "${GRT[$i]}" "$WORK/defsyms-$t.txt" || exit 1
done
echo "the Grasm runtime for ${TG[*]}: $(echo "$moved" | grep -c .) names moved, exported as families.txt lists; it calls none of the Rust runtime's"

# Links object `$2` with the Grasm runtime `$3` alone into `$d/$1.x`, by
# compiler/link; what it says, and its exit status when not 0, go to
# `$d/$1.link`, which the machine's half reads as a failed link.
glink() {
    "$LINK" -sysroot "${ROOT[$i]}" "$2" -interpose "$3" -dead_strip -strip -o "$d/$1.x" >"$d/$1.link" 2>&1 ||
        echo "compiler/link: exit $?" >>"$d/$1.link"
}

s3_collect
linux_filter_programs
external=0
declare -a EXTERNAL=() FAILED=() TIMEDOUT=()
: >"$WORK/batch.txt"
: >"$WORK/refs.txt"
for k in "${!S3_SRC[@]}"; do
    src=${S3_SRC[$k]}
    rule=${S3_RULE[$k]}
    case "$rule" in
    external:*)
        external=$((external + 1))
        EXTERNAL+=("$src")
        continue
        ;;
    esac
    for i in "${!TG[@]}"; do
        t=${TG[$i]}
        d="$WORK/p/$t/$k"
        mkdir -p "$d"
        if ! "$GRASM" build "$src" --native --target "${TRIPLE[$i]}" -o "$d/c.o" >"$d/c.log" 2>&1 || [ ! -f "$d/c.o" ]; then
            FAILED+=("$t $src — the Cranelift build for ${TRIPLE[$i]} fails: $(head -1 "$d/c.log")")
            continue
        fi
        "$NM" -u "$d/c.o" | awk -v k="$t $k" '$NF ~ /^grasm_/ { print k, $NF }' >>"$WORK/refs.txt"
        s=0
        if [ "$rule" = in ]; then
            if "$S3" "${S3_TIR[$k]}" -o "$d/s.o" --target "$t" >"$d/s3.log" 2>&1; then
                s=1
            else
                FAILED+=("$t $src — in the subset, and the backend does not compile it: $(head -1 "$d/s3.log")")
            fi
        fi
        # The links with the Grasm runtime alone, here: compiler/link runs
        # on the Mac.
        glink c.grasm "$d/c.o" "${GRT[$i]}"
        glink c.grasmp "$d/c.o" "${GRTP[$i]}"
        if [ "$s" = 1 ]; then
            glink s.grasm "$d/s.o" "${GRT[$i]}"
        fi
        printf '%s\t%s\t%s\t%s\n' "$i" "$k" "$d" "$s" >>"$WORK/batch.txt"
    done
done

# The machine's half, in one start of it: each object linked with each
# runtime and run, four programs at a time. A run past the limit exits 124.
{
    echo '#!/bin/bash'
    echo "cd '$PWD'"
    for i in "${!TG[@]}"; do
        echo "TG[$i]=${TG[$i]}; RRT[$i]='${RRT[$i]}'; RRTP[$i]='${RRTP[$i]}'"
    done
    cat <<EOF
X64_LD='$X64_LD'; X64_RUN='$X64_RUN'; XR='$X64_ROOT'; XL='$X64_LIB'
LIMIT=$S3_RUN_SECONDS
# with the Rust runtime: obj archive out
rust() {
    if [ "\${TG[\$i]}" = aarch64-linux ]; then
        gcc "\$1" "\$2" -lm -o "\$3" -Wl,--gc-sections
    else
        "\$X64_LD" -pie -z now -dynamic-linker /lib64/ld-linux-x86-64.so.2 "\$XR\$XL/Scrt1.o" "\$XR\$XL/crti.o" "\$1" "\$2" -L=\$XL -lm -lc "\$XR\$XL/libgcc_s.so.1" "\$XR\$XL/crtn.o" --gc-sections -o "\$3"
    fi
}
# links (with the Rust runtime) and runs one: name rust obj runtime; or runs
# one compiler/link linked on the Mac: name linked
one() {
    local n=\$1 how=\$2
    if [ "\$how" = rust ]; then
        if ! rust "\$3" "\$4" "\$d/\$n.x" >"\$d/\$n.link" 2>&1 || [ -s "\$d/\$n.link" ]; then
            echo "exit \$?" >>"\$d/\$n.link"
            echo 127 >"\$d/\$n.code"
            return
        fi
    elif [ -s "\$d/\$n.link" ] || [ ! -f "\$d/\$n.x" ]; then
        echo 127 >"\$d/\$n.code"
        return
    fi
    local run=""
    [ "\${TG[\$i]}" = x86_64-linux ] && run="\$X64_RUN"
    mkdir -p "\$d/tmp-\$n"
    timeout \$LIMIT $LINUX_ISOLATE "\$d/tmp-\$n" \$run "\$d/\$n.x" >"\$d/\$n.out" 2>"\$d/\$n.err" </dev/null
    echo \$? >"\$d/\$n.code"
    rm -f "\$d/\$n.x"
}
prog() {
    i=\$1; d=\$2; s=\$3
    one c.rust rust "\$d/c.o" "\${RRT[\$i]}"
    one c.grasm linked
    one c.rustp rust "\$d/c.o" "\${RRTP[\$i]}"
    one c.grasmp linked
    if [ "\$s" = 1 ]; then
        one s.rust rust "\$d/s.o" "\${RRT[\$i]}"
        one s.grasm linked
    fi
}
while IFS="\$(printf '\t')" read -r i k d s; do
    prog "\$i" "\$d" "\$s" &
    while [ "\$(jobs -pr | wc -l)" -ge 4 ]; do sleep 0.05; done
done <"$WORK/batch.txt"
wait
EOF
} >"$WORK/machine.sh"
chmod +x "$WORK/machine.sh"
linux_run "$WORK/machine.sh"

# Compares two runs: answers 0 the same, 1 different (why on stdout), 2 out
# of time.
pair() {
    local d=$1 a=$2 b=$3 ac bc
    for n in "$a" "$b"; do
        if [ -s "$d/$n.link" ]; then
            echo "$n does not link: $(head -2 "$d/$n.link" | tr '\n' ' ')"
            return 1
        fi
    done
    ac=$(cat "$d/$a.code")
    bc=$(cat "$d/$b.code")
    if [ "$ac" = 124 ] || [ "$bc" = 124 ]; then
        echo "TIMEOUT $a $ac $b $bc"
        return 2
    fi
    if cmp -s "$d/$a.out" "$d/$b.out" && [ "$ac" = "$bc" ] && cmp -s "$d/$a.err" "$d/$b.err"; then
        return 0
    fi
    echo "exit $ac ($a) vs $bc ($b)"
    diff "$d/$a.out" "$d/$b.out" | head -6 | sed 's/^/    /'
    diff "$d/$a.err" "$d/$b.err" | head -4 | sed 's/^/    /'
    return 1
}

declare -a C_SAME=() P_SAME=() S_IN=() S_SAME=() N=()
for i in "${!TG[@]}"; do C_SAME[$i]=0; P_SAME[$i]=0; S_IN[$i]=0; S_SAME[$i]=0; N[$i]=0; done
while IFS=$'\t' read -r i k d s; do
    t=${TG[$i]}
    src=${S3_SRC[$k]}
    N[$i]=$((N[$i] + 1))
    for which in c p s; do
        case $which in
        c) a=c.rust; b=c.grasm ;;
        p) a=c.rustp; b=c.grasmp ;;
        s) [ "$s" = 1 ] || continue; a=s.rust; b=s.grasm; S_IN[$i]=$((S_IN[$i] + 1)) ;;
        esac
        why=$(pair "$d" "$a" "$b")
        r=$?
        if [ "$r" = 0 ] && [ "$which" = p ] && grep -q 'use after free' "$d/$a.err" "$d/$b.err"; then
            r=1
            why="a use after free under the poisoned runtime: $(grep -h 'use after free' "$d/$a.err" "$d/$b.err" | head -1)"
        fi
        if [ "$r" = 0 ]; then
            case $which in
            c) C_SAME[$i]=$((C_SAME[$i] + 1)) ;;
            p) P_SAME[$i]=$((P_SAME[$i] + 1)) ;;
            s) S_SAME[$i]=$((S_SAME[$i] + 1)) ;;
            esac
        elif [ "$r" = 2 ]; then
            TIMEDOUT+=("$t $src ($which) — $why")
        else
            FAILED+=("$t $src ($which) — $why")
        fi
    done
done <"$WORK/batch.txt"

echo
for i in "${!TG[@]}"; do
    t=${TG[$i]}
    distinct=$(awk -v t="$t" '$1 == t { print $3 }' "$WORK/refs.txt" | LC_ALL=C sort -u)
    nd=$(echo "$distinct" | grep -c .)
    nr=$(awk -v t="$t" '$1 == t' "$WORK/refs.txt" | wc -l | tr -d ' ')
    bound=$(sed -n 's/^--defsym=\(grasm_[^=]*\)=.*/\1/p' "$WORK/defsyms-$t.txt" | LC_ALL=C sort -u)
    unbound=$(LC_ALL=C comm -23 <(echo "$distinct") <(echo "$bound") | tr '\n' ' ')
    echo "$t: programs with a main: ${N[$i]}   external set aside: $external"
    echo "$t: Cranelift objects: identical: ${C_SAME[$i]}   poisoned runtime, agreeing with no use after free: ${P_SAME[$i]}"
    echo "$t: Grasm backend objects: in the subset: ${S_IN[$i]}   identical: ${S_SAME[$i]}"
    echo "$t: runtime names the programs reference: $nd distinct, all taken from Grasm; per program, $nr references bound to Grasm"
    if [ -n "$unbound" ]; then
        FAILED+=("$t — names the programs reference that the Grasm runtime does not define: $unbound")
    fi
    if [ "${C_SAME[$i]}" -ne "${N[$i]}" ] || [ "${P_SAME[$i]}" -ne "${N[$i]}" ] || [ "${S_SAME[$i]}" -ne "${S_IN[$i]}" ]; then
        [ ${#FAILED[@]} -gt 0 ] || [ ${#TIMEDOUT[@]} -gt 0 ] || FAILED+=("$t — the counts do not add up")
    fi
    if [ -z "${DIRS:-}" ] && [ "${S_IN[$i]}" -lt "$S3_SUBSET_FLOOR" ]; then
        FAILED+=("$t — only ${S_IN[$i]} programs in the subset; the floor is $S3_SUBSET_FLOOR")
    fi
    [ "$nd" -gt 0 ] || FAILED+=("$t — no program references a runtime name: nothing was compared")
done
echo "with the Grasm runtime, linked by compiler/link (-interpose); with the Rust runtime, by $(linux_run sh -c 'ld --version | head -1')"
if [ "$external" -gt 0 ]; then
    corpus_external_covered "${EXTERNAL[@]}" || exit 1
fi
if [ ${#TIMEDOUT[@]} -gt 0 ]; then
    echo
    echo "TIMED OUT after ${S3_RUN_SECONDS}s (not agreement):"
    printf '  %s\n' "${TIMEDOUT[@]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}" | head -80
    exit 1
fi
[ ${#TIMEDOUT[@]} -eq 0 ] || exit 1
exit 0
