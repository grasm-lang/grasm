#!/usr/bin/env bash
# The Grasm backend's x86-64 objects are objects the tools accept, and the
# same TIR makes the same bytes (docs/self-hosting.md §6.16, stage S7), as
# `objtest` holds the arm64 ones. For every corpus program in the subset, the
# object `s3 --target x86_64` writes must:
#
#   * be bit-identical when compiled a second time, into another path;
#   * be read by `otool -l`, with LC_SEGMENT_64 holding __text,
#     LC_BUILD_VERSION, LC_SYMTAB and LC_DYSYMTAB, and say it is x86_64;
#   * be read by `nm`, defining `_main` as an external;
#   * carry a relocation ending every instruction the linker must complete --
#     a BRANCH on each `call` to a symbol, a SIGNED on each `leaq
#     sym(%rip)`, a GOT_LOAD or TLV on each `movq sym@...(%rip)` -- and none
#     anywhere else, with `__data`, `__thread_vars` and the DWARF checked as
#     for arm64 (`scripts/x64_macho.py relocs`);
#   * link with `cc -arch x86_64` and the x86-64 runtime, with nothing on the
#     linker's output;
#   * carry a DWARF line table `dwarfdump --verify` finds no error in;
#   * map, by `dwarfdump --debug-line`, every instruction that loads the
#     address of a string literal written once in the program to the file
#     and line the literal is on (`x64_macho.py lines`), the count held to a
#     floor.
#
# The runtime's objects (`s3 --runtime --target x86_64`) are held to the same
# once the runtime compiles for x86-64 (step four).
#
# `S3_STOP_AT_FIRST=1` stops at the first program with a problem (the
# controls, `scripts/x64controls.sh`). `S3_RT_X64=` names the x86-64 runtime,
# otherwise built as `crosstest` builds it.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
. "$(dirname "$0")/lib/corpus.sh"
CORPUS_DIRS=${DIRS:-$CORPUS_DIRS}
corpus_require

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
. "$(dirname "$0")/lib/s3.sh"
s3_driver
TARGET=x86_64-apple-darwin
if [ -z "${S3_RT_X64:-}" ]; then
    if [ "${GRASM_NO_BUILD:-}" != 1 ] && ! cargo build --release -q -p grasm_rt_abi --target "$TARGET"; then
        echo "error: the runtime for $TARGET does not build"
        exit 1
    fi
    S3_RT_X64=./target/$TARGET/release/libgrasm_rt_abi.a
fi
[ -f "$S3_RT_X64" ] || { echo "error: $S3_RT_X64 is missing"; exit 1; }
s3_collect
STOP=${S3_STOP_AT_FIRST:-}
stopped=0

checked=0
identical=0
problems=0
lines_checked=0
lines_repeated=0
declare -a FAILED=()
mkdir -p "$WORK/a" "$WORK/b/deeper"

for k in "${!S3_SRC[@]}"; do
    if [ -n "$STOP" ] && [ "$problems" -gt 0 ]; then
        stopped=1
        break
    fi
    [ "${S3_RULE[$k]}" = "in" ] || continue
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    checked=$((checked + 1))
    a="$WORK/a/p.o"
    b="$WORK/b/deeper/q.o"
    rm -f "$a" "$b"
    if ! "$S3" "$tir" -o "$a" --target x86_64 >"$WORK/s3.log" 2>&1 || ! "$S3" "$tir" -o "$b" --target x86_64 >>"$WORK/s3.log" 2>&1; then
        problems=$((problems + 1))
        FAILED+=("$src — the backend does not compile it: $(head -1 "$WORK/s3.log")")
        continue
    fi
    bad=""
    if cmp -s "$a" "$b"; then
        identical=$((identical + 1))
    else
        bad="$bad; two compilations differ"
    fi
    if otool -l "$a" >"$WORK/otool.txt" 2>&1; then
        for want in LC_SEGMENT_64 LC_BUILD_VERSION LC_SYMTAB LC_DYSYMTAB "sectname __text"; do
            if ! grep -c "$want" "$WORK/otool.txt" >/dev/null; then
                bad="$bad; otool -l shows no $want"
            fi
        done
    else
        bad="$bad; otool -l refuses it: $(head -1 "$WORK/otool.txt")"
    fi
    if nm "$a" >"$WORK/nm.txt" 2>&1; then
        if ! grep -cE '^[0-9a-f]+ T _main$' "$WORK/nm.txt" >/dev/null; then
            bad="$bad; nm shows no external _main"
        fi
    else
        bad="$bad; nm refuses it: $(head -1 "$WORK/nm.txt")"
    fi
    if ! otool -hv "$a" 2>/dev/null | grep -c X86_64 >/dev/null; then
        bad="$bad; otool -h does not say X86_64"
    fi
    if ! python3 scripts/x64_macho.py relocs "$a" >"$WORK/relocs.txt" 2>&1; then
        bad="$bad; relocations: $(head -2 "$WORK/relocs.txt" | tr '\n' ' ')"
    fi
    if ! dwarfdump --verify "$a" >"$WORK/verify.txt" 2>&1 || ! grep -cx "No errors." "$WORK/verify.txt" >/dev/null; then
        bad="$bad; dwarfdump --verify: $(grep -iE 'error|warning' "$WORK/verify.txt" | head -2 | tr '\n' ' ')"
    fi
    if python3 scripts/x64_macho.py lines "$a" "$tir" >"$WORK/lines.txt" 2>&1; then
        n=$(sed -n 's/.*literal loads checked: \([0-9]*\).*/\1/p' "$WORK/lines.txt" | tail -1)
        lines_checked=$((lines_checked + ${n:-0}))
        n=$(sed -n 's/.*of literals loaded more than once: \([0-9]*\).*/\1/p' "$WORK/lines.txt" | tail -1)
        lines_repeated=$((lines_repeated + ${n:-0}))
    else
        bad="$bad; line table: $(head -2 "$WORK/lines.txt" | tr '\n' ' ')"
    fi
    if cc -arch x86_64 "$a" "$S3_RT_X64" -o "$WORK/prog" -Wl,-dead_strip >"$WORK/link.txt" 2>&1; then
        if [ -s "$WORK/link.txt" ]; then
            bad="$bad; the linker said: $(head -2 "$WORK/link.txt" | tr '\n' ' ')"
        fi
    else
        bad="$bad; the linker refuses it: $(head -2 "$WORK/link.txt" | tr '\n' ' ')"
    fi
    if [ -n "$bad" ]; then
        problems=$((problems + 1))
        FAILED+=("$src${bad}")
    fi
done

echo "objects: $checked   reproducible: $identical   literal loads on their lines: $lines_checked (of literals loaded more than once, as inlined copies are: $lines_repeated)   with problems: $problems"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
fi
if [ "$stopped" = 1 ]; then
    echo
    echo "stopped at the first failure (S3_STOP_AT_FIRST); the counts above are partial"
    exit 1
fi
[ "$problems" -eq 0 ] || exit 1
[ "$identical" -eq "$checked" ] || exit 1
if [ -z "${DIRS:-}" ] && [ "$checked" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $checked objects were checked; the subset's floor is $S3_SUBSET_FLOOR"
    exit 1
fi
# The line tables' sample, held to a floor as `objtest`'s is: far fewer than
# arm64's 100,000 would be a check that stopped finding its sample.
if [ -z "${DIRS:-}" ] && [ "$lines_checked" -lt 100000 ]; then
    echo "only $lines_checked literal loads were looked up in the line tables; the floor is 100000"
    exit 1
fi
[ "$lines_checked" -gt 0 ] || { echo "no instruction was looked up in a line table"; exit 1; }
[ "$checked" -gt 0 ] || { echo "nothing was checked"; exit 1; }
exit 0
