#!/usr/bin/env bash
# The Grasm backend for Linux (docs/self-hosting.md §6.18, stage S7) must
# print and exit exactly as the Cranelift build for the same triple does,
# and as the interpreter does, on every corpus program in the subset, on
# arm64 and x86-64 Linux -- `crosstest` with the Grasm backend as the
# compiler, run in the OrbStack machine (x86-64 through Rosetta).
#
# For each corpus program with a `main` (the same TIR and the same subset
# rule as `s3subset`), for each of `aarch64-linux` and `x86_64-linux`:
#
#   * membership -- the backend compiles every program the rule has in and
#     refuses (exit 3) every one it has out;
#   * agreement -- `s3 --target <t>` compiles it, and it is linked with the
#     Grasm runtime built for Linux (`compiler/rt` with `os/linux.gr`,
#     `s3 --runtime --target <t>`) and `compiler/link`'s `_start`, glibc's
#     `libm.so.6` and `libc.so.6`, by GNU ld -- until `compiler/link` links
#     ELF (step four of §6.18, this gate's recorded gap) -- with each of the
#     program's runtime names bound to the Grasm runtime's, as `link
#     -interpose` binds them. The reference is the Cranelift build for the
#     triple (`grasm build --native --target <triple>`'s object) linked with
#     the Rust runtime built for the triple by the machine's toolchain (gcc;
#     for x86-64, the cross ld, crt files and glibc of `GRASM_X64_ROOT`).
#     Both run in the machine from the checkout, stdin closed, x86-64 ones
#     through the cross glibc's dynamic loader, each with a /tmp of its own
#     (`scripts/lib/isolate.py`: the machine's /tmp is the owner's, and
#     fifteen of the programs write there). stdout, the exit status and
#     stderr must be the reference's, and stdout and the exit status
#     `grasm run`'s on the Mac, whose stderr is compared with its panic
#     trace taken out ([E-PANIC-TRACE]).
#
# Where both Linux builds agree with each other and differ from the
# interpreter on the Mac -- glibc's libm and Apple's differ in the last
# place on some arguments, glibc's `strerror` words some errors otherwise --
# the program must be listed, with the reason, in `scripts/elfsubset_known.txt`
# for that triple, and is counted apart; a listed program that agrees with
# the interpreter is reported, so the list cannot go stale. A program that
# links a system library is set aside, as `s3subset` sets it aside.
#
# Needs the OrbStack machine and `GRASM_X64_ROOT` (scripts/lib/linux.sh);
# without them it does not run (exit 77). `S3=` tests that driver, `LINK=`
# that linker; `S3_STOP_AT_FIRST=1` stops at the first failure (the
# controls, `scripts/elfcontrols.sh`); `ELF_TARGETS=` limits the targets,
# `ELF_PROGRAMS=<regex>` the programs (no floor then);
# `ELF_RT_A64=`/`ELF_RT_X64=` name the Rust runtimes, otherwise built into
# `${CARGO_TARGET_DIR:-./target}`.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. scripts/lib/linux.sh
linux_require
. scripts/lib/fresh.sh
fresh_or_stop "./target/release/grasm"
. scripts/lib/trace.sh
. scripts/lib/corpus.sh
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require

WORK=$(linux_workdir)
trap 'rm -rf "$WORK"' EXIT
linux_x64_root
linux_isolate_check
. scripts/lib/s3.sh
s3_driver
. scripts/lib/link.sh
link_driver
KNOWN=scripts/elfsubset_known.txt
TARGETS=${ELF_TARGETS:-aarch64-linux x86_64-linux}
# shellcheck disable=SC2206
TG=($TARGETS)
CARGO_OUT=${CARGO_TARGET_DIR:-./target}
declare -a TRIPLE=() RUST_RT=() GRASM_RT=() START=()
for i in "${!TG[@]}"; do
    t=${TG[$i]}
    case $t in
    aarch64-linux) triple=aarch64-unknown-linux-gnu; var=ELF_RT_A64; arch=aarch64 ;;
    x86_64-linux) triple=x86_64-unknown-linux-gnu; var=ELF_RT_X64; arch=x86_64 ;;
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
external=0
declare -a IN_SUBSET=() OUT_SUBSET=() PASS=() FAIL=() TIMEDOUT=() KNOWN_N=()
declare -a FAILED=() TIMEDOUT_L=() EXTERNAL=() KNOWN_L=()
for i in "${!TG[@]}"; do
    IN_SUBSET[$i]=0; OUT_SUBSET[$i]=0; PASS[$i]=0; FAIL[$i]=0; TIMEDOUT[$i]=0; KNOWN_N[$i]=0
done
: >"$WORK/batch.txt"
mkdir -p "$WORK/i"

# The Mac's half: compile, and run the interpreter once per program.
for k in "${!S3_SRC[@]}"; do
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    rule=${S3_RULE[$k]}
    case "$rule" in
    external:*)
        external=$((external + 1))
        EXTERNAL+=("$src")
        continue
        ;;
    esac
    want_run=0
    for i in "${!TG[@]}"; do
        t=${TG[$i]}
        d="$WORK/p/$t/$k"
        mkdir -p "$d"
        "$S3" "$tir" -o "$d/s.o" --target "$t" >"$d/s3.out" 2>"$d/s3.err"
        code=$?
        if [ "$rule" != "in" ]; then
            OUT_SUBSET[$i]=$((OUT_SUBSET[$i] + 1))
            if [ "$code" != 3 ]; then
                FAIL[$i]=$((FAIL[$i] + 1))
                FAILED+=("$t $src — outside the subset ($rule), but the backend answered $code, not a refusal: $(head -1 "$d/s3.err")")
            fi
            continue
        fi
        IN_SUBSET[$i]=$((IN_SUBSET[$i] + 1))
        if [ "$code" != 0 ]; then
            FAIL[$i]=$((FAIL[$i] + 1))
            FAILED+=("$t $src — in the subset, and the backend answered $code: $(head -3 "$d/s3.err" | tr '\n' ' ')")
            continue
        fi
        if ! "$GRASM" build "$src" --native --target "${TRIPLE[$i]}" -o "$d/c.o" >"$d/c.log" 2>&1 || [ ! -f "$d/c.o" ]; then
            FAIL[$i]=$((FAIL[$i] + 1))
            FAILED+=("$t $src — the Cranelift build for ${TRIPLE[$i]} fails: $(head -1 "$d/c.log")")
            continue
        fi
        printf '%s\t%s\t%s\n' "$i" "$k" "$d" >>"$WORK/batch.txt"
        want_run=1
    done
    if [ "$want_run" = 1 ]; then
        $RUN_LIMIT "$GRASM" run "$src" >"$WORK/i/$k.out" 2>"$WORK/i/$k.err" </dev/null
        echo $? >"$WORK/i/$k.code"
    fi
    if [ -n "$STOP" ] && [ "${#FAILED[@]}" -gt 0 ]; then
        stopped=1
        break
    fi
done

# The machine's half, in one start of it: each program linked twice and run
# twice, four at a time. A run past the limit exits 124.
{
    echo '#!/bin/bash'
    echo "cd '$PWD'"
    for i in "${!TG[@]}"; do
        echo "TG[$i]=${TG[$i]}; RRT[$i]='${RUST_RT[$i]}'; GRT[$i]='${GRASM_RT[$i]}'; ST[$i]='${START[$i]}'; DS[$i]='$WORK/defsyms-${TG[$i]}.txt'"
    done
    cat <<EOF
X64_LD='$X64_LD'; X64_RUN='$X64_RUN'; XR='$X64_ROOT'; XL='$X64_LIB'
LIMIT=$S3_RUN_SECONDS
FLAGS="-pie -z now -z relro --hash-style=gnu --gc-sections --build-id=none"
one() {
    i=\$1; d=\$2
    if [ "\${TG[\$i]}" = aarch64-linux ]; then
        ld \$FLAGS -dynamic-linker /lib/ld-linux-aarch64.so.1 "\${ST[\$i]}" "\$d/s.o" "\${GRT[\$i]}" /lib/aarch64-linux-gnu/libm.so.6 /lib/aarch64-linux-gnu/libc.so.6 @"\${DS[\$i]}" -o "\$d/s.x" >"\$d/s.link" 2>&1 || echo "exit \$?" >>"\$d/s.link"
        gcc "\$d/c.o" "\${RRT[\$i]}" -lm -o "\$d/c.x" -Wl,--gc-sections >"\$d/c.link" 2>&1 || echo "exit \$?" >>"\$d/c.link"
        run=""
    else
        "\$X64_LD" \$FLAGS -dynamic-linker /lib64/ld-linux-x86-64.so.2 -rpath-link "\$XR\$XL" "\${ST[\$i]}" "\$d/s.o" "\${GRT[\$i]}" "\$XR\$XL/libm.so.6" "\$XR\$XL/libc.so.6" @"\${DS[\$i]}" -o "\$d/s.x" >"\$d/s.link" 2>&1 || echo "exit \$?" >>"\$d/s.link"
        "\$X64_LD" -pie -z now -dynamic-linker /lib64/ld-linux-x86-64.so.2 "\$XR\$XL/Scrt1.o" "\$XR\$XL/crti.o" "\$d/c.o" "\${RRT[\$i]}" -L=\$XL -lm -lc "\$XR\$XL/libgcc_s.so.1" "\$XR\$XL/crtn.o" --gc-sections -o "\$d/c.x" >"\$d/c.link" 2>&1 || echo "exit \$?" >>"\$d/c.link"
        run="\$X64_RUN"
    fi
    for w in s c; do
        if [ -s "\$d/\$w.link" ]; then
            echo 127 >"\$d/\$w.code"
            continue
        fi
        mkdir -p "\$d/tmp-\$w"
        timeout \$LIMIT $LINUX_ISOLATE "\$d/tmp-\$w" \$run "\$d/\$w.x" >"\$d/\$w.out" 2>"\$d/\$w.err" </dev/null
        echo \$? >"\$d/\$w.code"
        rm -f "\$d/\$w.x"
    done
}
while IFS="\$(printf '\t')" read -r i k d; do
    one "\$i" "\$d" &
    while [ "\$(jobs -pr | wc -l)" -ge 4 ]; do sleep 0.05; done
done <"$WORK/batch.txt"
wait
EOF
} >"$WORK/machine.sh"
chmod +x "$WORK/machine.sh"
if [ "$stopped" = 0 ]; then
    linux_run "$WORK/machine.sh"
fi

# Agreement, and the programs listed as differing from the interpreter.
while IFS=$'\t' read -r i k d; do
    [ "$stopped" = 1 ] && break
    t=${TG[$i]}
    src=${S3_SRC[$k]}
    if [ -s "$d/s.link" ] || [ -s "$d/c.link" ]; then
        FAIL[$i]=$((FAIL[$i] + 1))
        FAILED+=("$t $src — does not link: $(head -2 "$d/s.link" "$d/c.link" 2>/dev/null | tr '\n' ' ')")
        continue
    fi
    s_code=$(cat "$d/s.code" 2>/dev/null)
    c_code=$(cat "$d/c.code" 2>/dev/null)
    i_code=$(cat "$WORK/i/$k.code")
    if [ "$s_code" = 124 ] || [ "$c_code" = 124 ] || [ "$i_code" = 124 ]; then
        TIMEDOUT[$i]=$((TIMEDOUT[$i] + 1))
        TIMEDOUT_L+=("$t $src — Cranelift $c_code, s3 $s_code, interpreter $i_code")
        continue
    fi
    i_diag=$(strip_trace "$(cat "$WORK/i/$k.err")")
    s_diag=$(cat "$d/s.err")
    same_ref=0
    same_interp=0
    if cmp -s "$d/c.out" "$d/s.out" && [ "$c_code" = "$s_code" ] && cmp -s "$d/c.err" "$d/s.err"; then
        same_ref=1
    fi
    if cmp -s "$WORK/i/$k.out" "$d/s.out" && [ "$i_code" = "$s_code" ] && [ "$i_diag" == "$s_diag" ]; then
        same_interp=1
    fi
    listed=$(awk -F'\t' -v t="$t" -v s="$src" '$1 == t && $2 == s { print $3 }' "$KNOWN" 2>/dev/null)
    if [ "$same_ref" = 1 ] && [ "$same_interp" = 1 ]; then
        PASS[$i]=$((PASS[$i] + 1))
        if [ -n "$listed" ]; then
            FAIL[$i]=$((FAIL[$i] + 1))
            FAILED+=("$t $src — listed in $KNOWN ($listed), but agrees with the interpreter: take it off the list")
        fi
    elif [ "$same_ref" = 1 ] && [ -n "$listed" ]; then
        KNOWN_N[$i]=$((KNOWN_N[$i] + 1))
        KNOWN_L+=("$t $src — $listed")
    else
        FAIL[$i]=$((FAIL[$i] + 1))
        FAILED+=("$t $src — exit $c_code (Cranelift) vs $s_code (s3) vs $i_code (interpreter)$([ "$same_ref" = 1 ] && echo '; the Linux builds agree, and the program is not in '"$KNOWN")")
        echo "MISMATCH: $t $src (exit $c_code Cranelift, $s_code s3, $i_code interpreter)"
        diff "$d/c.out" "$d/s.out" | head -8 | sed 's/^/  /'
        diff "$d/c.err" "$d/s.err" | head -6 | sed 's/^/  /'
        diff "$WORK/i/$k.out" "$d/s.out" | head -4 | sed 's/^/  interp: /'
        if [ -n "$STOP" ]; then
            stopped=1
        fi
    fi
done <"$WORK/batch.txt"

echo
for i in "${!TG[@]}"; do
    echo "${TG[$i]}: subset: ${IN_SUBSET[$i]} in, ${OUT_SUBSET[$i]} out, $external external, of ${#S3_SRC[@]} programs with a main"
    echo "${TG[$i]}: identical: ${PASS[$i]}   mismatched: ${FAIL[$i]}   timed out: ${TIMEDOUT[$i]}   agreeing with the Cranelift build and not the interpreter, as listed: ${KNOWN_N[$i]}"
done
echo "linked by GNU ld ($(linux_run sh -c 'ld --version | head -1')), the Grasm runtime's names bound by --defsym; glibc $(linux_run sh -c 'ldd --version | head -1 | sed "s/.* //"')"
if [ "$external" -gt 0 ]; then
    corpus_external_covered "${EXTERNAL[@]}" || exit 1
fi
if [ ${#KNOWN_L[@]} -gt 0 ]; then
    echo
    echo "agreeing with the Cranelift build for the triple, not with the interpreter on the Mac (listed in $KNOWN):"
    printf '  %s\n' "${KNOWN_L[@]}"
fi
if [ ${#S3_NOTIR[@]} -gt 0 ]; then
    echo
    echo "emit no TIR:"
    printf '  %s\n' "${S3_NOTIR[@]}"
fi
if [ ${#TIMEDOUT_L[@]} -gt 0 ]; then
    echo
    echo "TIMED OUT after ${S3_RUN_SECONDS}s compiled or ${RUN_LIMIT_SECONDS}s interpreted (not agreement):"
    printf '  %s\n' "${TIMEDOUT_L[@]}"
fi
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
[ ${#FAILED[@]} -eq 0 ] && [ ${#TIMEDOUT_L[@]} -eq 0 ] && [ ${#S3_NOTIR[@]} -eq 0 ] || exit 1
for i in "${!TG[@]}"; do
    if [ $((IN_SUBSET[$i] + OUT_SUBSET[$i] + external)) -ne ${#S3_SRC[@]} ]; then
        echo "${TG[$i]}: the counts do not add up"
        exit 1
    fi
    if [ $((PASS[$i] + KNOWN_N[$i])) -ne "${IN_SUBSET[$i]}" ]; then
        echo "${TG[$i]}: $((PASS[$i] + KNOWN_N[$i])) of ${IN_SUBSET[$i]} programs in the subset were accounted for"
        exit 1
    fi
    if [ -z "${DIRS:-}" ] && [ "${IN_SUBSET[$i]}" -lt "$S3_SUBSET_FLOOR" ]; then
        echo "${TG[$i]}: only ${IN_SUBSET[$i]} programs are in the subset; the floor is $S3_SUBSET_FLOOR"
        exit 1
    fi
    [ "${PASS[$i]}" -gt 0 ] || { echo "${TG[$i]}: nothing was compared"; exit 1; }
done
exit 0
