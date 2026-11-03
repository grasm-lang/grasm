#!/usr/bin/env bash
# `linkdiff` for x86-64 (docs/self-hosting.md §6.16, stage S7): the Grasm
# linker against ld-prime on the same x86-64 objects.
#
# For every fourth program of the subset (`scripts/s3_subset.py`) and every
# one with an `export fn` or a thread-local cell, two links each: the
# Cranelift x86-64 object (`grasm build --native --target x86_64-apple-darwin`)
# and the Grasm backend's (`s3 --target x86_64`), each with the Rust runtime
# built for x86_64-apple-darwin, `-dead_strip`:
#
#   * byte for byte: `compiler/link -strip -tool_version <ld's>` against
#     `cc -arch x86_64` followed by `strip -no_uuid`, as `linkdiff` holds
#     arm64 links to what the native build ships;
#   * unstripped, against `cc -arch x86_64`: `dyld_info -fixups -exports`,
#     `nm -m` and `otool -l` the same, minus what `linkdiff` explains for
#     arm64 (the UUID, ld's tool entry, the symbol table's size and what
#     follows it, ld's debug map), and every section and __LINKEDIT blob the
#     same bytes (`scripts/link_sections.py`);
#   * the C fixtures (`scripts/fixtures/link/`, `cc -arch x86_64 -c`) and the
#     whole x86-64 runtime -- all its members as objects after a C `main`,
#     without -dead_strip -- the same comparisons, the fixtures' symbol
#     tables included.
#
# Neither linker signs an x86-64 executable (ld signs none; §6.16).
# Counts are derived and asserted. `LINK=`, `S3=` and `LINK_PROGRAMS=<regex>`
# as for `scripts/linkdiff.sh`; `S3_RT_X64=` the runtime archive;
# `S3_STOP_AT_FIRST=1` stops at the first difference (the controls).
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
. "$(dirname "$0")/lib/link.sh"
link_driver
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
LD_VERSION=$(ld -v 2>&1 | head -1 | sed -E 's/.*ld-([0-9.]+).*/\1/')
case "$LD_VERSION" in
[0-9]*) ;;
*)
    echo "error: cannot read ld's version from 'ld -v'"
    exit 1
    ;;
esac
STOP=${S3_STOP_AT_FIRST:-}

# otool -l without what legitimately differs, as in `scripts/linkdiff.sh`.
otool_norm() {
    otool -l "$1" | sed 1d | awk '
        /^Load command/ { next }
        /cmd LC_UUID/ { skip_uuid = 1 }
        skip_uuid && /uuid/ { skip_uuid = 0; next }
        /cmd / { inbv = 0 }
        /cmd LC_BUILD_VERSION/ { inbv = 1 }
        inbv && /^ *cmdsize/ { next }
        inbv && /^ *ntools/ { next }
        inbv && /^ *tool / { next }
        inbv && /^ *version / { next }
        /cmd / { cmd = $2 }
        /sizeofcmds/ { next }
        cmd == "LC_SYMTAB" && /symoff|nsyms|stroff|strsize/ { next }
        cmd == "LC_DYSYMTAB" && /ilocalsym|nlocalsym|iextdefsym|iundefsym|indirectsymoff/ { next }
        { print }
    ' | awk '
        /segname __LINKEDIT/ { le = 1 }
        le && /vmsize|filesize/ { next }
        /segname / && !/__LINKEDIT/ { le = 0 }
        { print }
    '
}

dyld_norm() {
    dyld_info -fixups -exports "$1" 2>&1 | sed 1d
}

declare -a FAILED=()

check_sections() { # LD_OUT OURS NAME [--symbols]
    if python3 scripts/link_sections.py "$1" "$2" ${4:-} >"$WORK/s.log" 2>&1; then
        return 0
    fi
    FAILED+=("$3 — sections differ: $(grep -E 'DIFFERS|missing|differ in' "$WORK/s.log" | head -3 | tr '\n' ' ')")
    return 1
}

check_tools() { # LD_OUT OURS NAME
    local ref=$1 ours=$2 name=$3 bad=0
    if ! diff <(dyld_norm "$ref") <(dyld_norm "$ours") >"$WORK/d.diff"; then
        FAILED+=("$name — dyld_info -fixups -exports differ: $(head -4 "$WORK/d.diff" | tr '\n' ' ')")
        bad=1
    fi
    if ! diff <(nm -m "$ref") <(nm -m "$ours") >"$WORK/n.diff"; then
        FAILED+=("$name — nm -m differs: $(head -4 "$WORK/n.diff" | tr '\n' ' ')")
        bad=1
    fi
    if ! diff <(otool_norm "$ref") <(otool_norm "$ours") >"$WORK/o.diff"; then
        FAILED+=("$name — otool -l differs: $(head -6 "$WORK/o.diff" | tr '\n' ' ')")
        bad=1
    fi
    return $bad
}

# One object, linked both ways: byte for byte stripped, and unstripped by the
# tools and sections. Counts into `identical` and `compared`.
both() { # OBJ NAME
    local obj=$1 name=$2
    rm -rf "$WORK/ld" "$WORK/st" "$WORK/s" "$WORK/u"
    mkdir -p "$WORK/ld" "$WORK/st" "$WORK/s" "$WORK/u"
    if ! cc -arch x86_64 "$obj" "$S3_RT_X64" -Wl,-dead_strip -o "$WORK/ld/p" >"$WORK/cc.log" 2>&1; then
        FAILED+=("$name — cc does not link it: $(head -2 "$WORK/cc.log" | tr '\n' ' ')")
        return
    fi
    cp "$WORK/ld/p" "$WORK/st/p"
    if ! strip -no_uuid "$WORK/st/p" 2>"$WORK/strip.log"; then
        FAILED+=("$name — strip refuses ld's output: $(head -1 "$WORK/strip.log")")
        return
    fi
    if ! "$LINK" "$obj" "$S3_RT_X64" -dead_strip -strip -tool_version "$LD_VERSION" -o "$WORK/s/p" >"$WORK/link.log" 2>&1; then
        FAILED+=("$name — compiler/link failed: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
        return
    fi
    if cmp -s "$WORK/st/p" "$WORK/s/p"; then
        identical=$((identical + 1))
    else
        FAILED+=("$name — not the bytes ld and strip make (first difference at byte $(cmp "$WORK/st/p" "$WORK/s/p" | awk '{print $5}' | tr -d ,))")
    fi
    if ! "$LINK" "$obj" "$S3_RT_X64" -dead_strip -o "$WORK/u/p" >"$WORK/link.log" 2>&1; then
        FAILED+=("$name — compiler/link failed unstripped: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
        return
    fi
    if check_tools "$WORK/ld/p" "$WORK/u/p" "$name" && check_sections "$WORK/ld/p" "$WORK/u/p" "$name"; then
        compared=$((compared + 1))
    fi
}

# The C fixtures, for x86-64.
fx=0
for entry in "${LINK_FIXTURES[@]}"; do
    name=${entry%%|*}
    if ! link_fixture_build "$name" x86_64 >"$WORK/fx.log" 2>&1; then
        FAILED+=("fixture $name — cc does not build it for x86_64: $(head -2 "$WORK/fx.log" | tr '\n' ' ')")
        continue
    fi
    mkdir -p "$WORK/fx/$name/o"
    if ! "$LINK" "${FX_OBJS[@]}" "${FX_FLAGS[@]}" -o "$WORK/fx/$name/o/ref" >"$WORK/link.log" 2>&1; then
        FAILED+=("fixture $name — compiler/link failed: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
        continue
    fi
    if check_tools "$WORK/fx/$name/ref" "$WORK/fx/$name/o/ref" "fixture $name" &&
        check_sections "$WORK/fx/$name/ref" "$WORK/fx/$name/o/ref" "fixture $name" --symbols; then
        fx=$((fx + 1))
    fi
done

# The whole x86-64 runtime: every member, after a C `main`, without
# -dead_strip.
whole=0
mkdir -p "$WORK/rt/m" "$WORK/rt/o"
printf 'int main(void) { return 0; }\n' >"$WORK/rt/main.c"
members=()
while IFS= read -r m; do
    case "$m" in
    __.SYMDEF*) ;;
    *) members+=("$WORK/rt/m/$m") ;;
    esac
done < <(ar t "$S3_RT_X64")
rt_abs="$(cd "$(dirname "$S3_RT_X64")" && pwd)/$(basename "$S3_RT_X64")"
if cc -arch x86_64 -c "$WORK/rt/main.c" -o "$WORK/rt/main.o" && (cd "$WORK/rt/m" && ar x "$rt_abs") &&
    cc -arch x86_64 "$WORK/rt/main.o" "${members[@]}" -o "$WORK/rt/whole" >"$WORK/rt.log" 2>&1; then
    if "$LINK" "$WORK/rt/main.o" "${members[@]}" -o "$WORK/rt/o/whole" >"$WORK/link.log" 2>&1; then
        if check_tools "$WORK/rt/whole" "$WORK/rt/o/whole" "the whole runtime" &&
            check_sections "$WORK/rt/whole" "$WORK/rt/o/whole" "the whole runtime"; then
            whole=1
        fi
    else
        FAILED+=("the whole runtime — compiler/link failed: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
    fi
else
    FAILED+=("the whole runtime — cc does not link its ${#members[@]} members: $(head -2 "$WORK/rt.log" | tr '\n' ' ')")
fi

considered=0
sampled=0
links=0
identical=0
compared=0
# The programs, after the fixtures and the runtime, which are quick: with
# S3_STOP_AT_FIRST a difference there ends the run before them.
for k in "${!S3_SRC[@]}"; do
    if [ -n "$STOP" ] && [ ${#FAILED[@]} -gt 0 ]; then
        break
    fi
    src=${S3_SRC[$k]}
    [ "${S3_RULE[$k]}" = in ] || continue
    if [ -n "${LINK_PROGRAMS:-}" ] && ! [[ "$src" =~ $LINK_PROGRAMS ]]; then
        continue
    fi
    considered=$((considered + 1))
    take=0
    [ -n "${LINK_PROGRAMS:-}" ] && take=1
    [ $((k % 4)) = 0 ] && take=1
    grep -cE '^[[:space:]]*(pub )?export fn|mem_tls' "$src" >/dev/null && take=1
    [ "$take" = 1 ] || continue
    sampled=$((sampled + 1))
    rm -f "$WORK/cl.o" "$WORK/p.o"
    if ! "$GRASM" build "$src" --native --target "$TARGET" -o "$WORK/cl" >"$WORK/build.log" 2>&1; then
        FAILED+=("$src — the Cranelift x86-64 build fails: $(head -1 "$WORK/build.log")")
        continue
    fi
    if ! "$S3" "${S3_TIR[$k]}" -o "$WORK/p.o" --target x86_64 >"$WORK/s3.log" 2>&1; then
        FAILED+=("$src — s3 --target x86_64 fails: $(head -1 "$WORK/s3.log")")
        continue
    fi
    links=$((links + 2))
    both "$WORK/cl.o" "$src (Cranelift)"
    both "$WORK/p.o" "$src (Grasm backend)"
done

echo
echo "target: $TARGET   sampled: $sampled of $considered programs, $links links   byte-identical to ld and strip: $identical   tools and sections agree: $compared   C fixtures agree: $fx of ${#LINK_FIXTURES[@]}   whole runtime (${#members[@]} members) agrees: $whole   (ld $LD_VERSION)"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
if [ "$identical" -ne "$links" ] || [ "$compared" -ne "$links" ] || [ "$fx" -ne ${#LINK_FIXTURES[@]} ] || [ "$whole" -ne 1 ]; then
    echo "not every link was compared"
    exit 1
fi
if [ -z "${LINK_PROGRAMS:-}" ] && [ "$sampled" -lt $(( considered / 4 )) ]; then
    echo "the sample is too small: $sampled of $considered"
    exit 1
fi
[ "$links" -gt 0 ] || { echo "nothing was linked"; exit 1; }
exit 0
