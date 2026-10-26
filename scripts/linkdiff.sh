#!/usr/bin/env bash
# The Grasm linker against ld-prime, on the same objects (docs/self-hosting.md,
# stage S4).
#
# For a sample of the corpus -- every fourth program with a `main`, and every
# one that has an `export fn`, uses a thread-local cell or links a system
# library -- the Cranelift backend's objects and link line are captured from
# `grasm build --native` (GRASM_LINKER=cc, `scripts/lib/linkcc/cc`), and:
#
#   * byte for byte: `compiler/link -strip`, told to write ld's tool entry in
#     LC_BUILD_VERSION (`-tool_version`, the one thing it does not write by
#     default), must give exactly the bytes the native build shipped -- ld,
#     then `strip -no_uuid`: every section, every fixup chain, the export
#     trie, function starts, symbol and string tables and the signature;
#   * unstripped, against `cc` with `-Wl,-dead_strip` and the same exports:
#     `dyld_info -fixups -exports` and `nm -m` must print the same, and
#     `otool -l` the same load commands, minus what legitimately differs:
#       - LC_UUID's value: ld hashes its output, debug map included, with MD5;
#         compiler/link hashes its own with SHA-256;
#       - LC_BUILD_VERSION's tool entry: ld names itself, compiler/link names
#         no tool (it is not ld);
#       - the symbol table's size and what follows it in __LINKEDIT (symoff..,
#         the local-symbol counts, the indirect table's and the signature's
#         offsets, __LINKEDIT's size): ld writes a debug map (N_OSO stabs)
#         for every object carrying DWARF -- the runtime's members, the Grasm
#         backend's objects -- and compiler/link writes none, since those
#         objects live in a temporary directory and an N_OSO records a path
#         and a time. Symbols themselves are compared by `nm -m`.
#   * the thread-local fixture (`scripts/fixtures/link/tlv_*.c`, built with
#     `cc -c`): the same three tool comparisons against `cc`'s link.
#
# Counts are derived and asserted: the sample is drawn from the same list
# `scripts/lib/s3.sh` collects, and every program in it is compared.
# `LINK=`, `LINK_EXTRA` and `LINK_PROGRAMS` as for `scripts/linktest.sh`.
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
s3_runtime
s3_collect
EXTRA=${LINK_EXTRA:-}
SDL=$(link_sdl_present)
LD_VERSION=$(ld -v 2>&1 | head -1 | sed -E 's/.*ld-([0-9.]+).*/\1/')
case "$LD_VERSION" in
[0-9]*) ;;
*)
    echo "error: cannot read ld's version from 'ld -v'"
    exit 1
    ;;
esac

# otool -l without what legitimately differs (see above).
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
        cmd == "LC_CODE_SIGNATURE" && /dataoff|datasize/ { next }
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

considered=0
sampled=0
identical=0
compared=0
declare -a FAILED=()

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

for k in "${!S3_SRC[@]}"; do
    src=${S3_SRC[$k]}
    rule=${S3_RULE[$k]}
    if [ -n "${LINK_PROGRAMS:-}" ] && ! [[ "$src" =~ $LINK_PROGRAMS ]]; then
        continue
    fi
    considered=$((considered + 1))
    take=0
    [ -n "${LINK_PROGRAMS:-}" ] && take=1
    [ $((k % 4)) = 0 ] && take=1
    grep -cE '^[[:space:]]*(pub )?export fn|mem_tls' "$src" >/dev/null && take=1
    case "$rule" in
    external:*) take=1 ;;
    esac
    [ "$take" = 1 ] || continue
    case "$rule" in
    external:*)
        if [ "$SDL" != yes ]; then
            echo "set aside (no SDL2): $src"
            continue
        fi
        ;;
    esac
    sampled=$((sampled + 1))
    if ! link_capture "$src" "$WORK/cl"; then
        FAILED+=("$src — the native build fails: $(head -1 "$WORK/cl.log")")
        continue
    fi
    if ! args=$(link_args "$WORK/cl.cap"); then
        FAILED+=("$src — its link line has a flag compiler/link does not take")
        continue
    fi
    IFS=$'\n' read -r -d '' -a argv <<<"$args"
    mkdir -p "$WORK/s" "$WORK/u" "$WORK/ld"
    # Byte for byte against what the native build shipped. The output's
    # name is the signature's identifier, so it is the build's: `cl`.
    # shellcheck disable=SC2086
    if ! "$LINK" "${argv[@]}" -strip -tool_version "$LD_VERSION" $EXTRA -o "$WORK/s/cl" >"$WORK/link.log" 2>&1; then
        FAILED+=("$src — compiler/link failed: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
        continue
    fi
    if cmp -s "$WORK/cl" "$WORK/s/cl"; then
        identical=$((identical + 1))
    else
        FAILED+=("$src — not the bytes the native build shipped (first difference at byte $(cmp "$WORK/cl" "$WORK/s/cl" | awk '{print $5}' | tr -d ,))")
    fi
    # Unstripped, against cc.
    ccargs=()
    skip=0
    while read -r kind val; do
        if [ "$skip" = 1 ]; then
            skip=0
            continue
        fi
        if [ "$kind" = OBJ ]; then
            ccargs+=("$WORK/cl.cap/$val")
        elif [ "$val" = -o ]; then
            skip=1
        else
            ccargs+=("$val")
        fi
    done <"$WORK/cl.cap/args"
    if ! cc "${ccargs[@]}" -o "$WORK/ld/p" >"$WORK/cc.log" 2>&1; then
        FAILED+=("$src — cc does not link the captured objects again: $(head -2 "$WORK/cc.log" | tr '\n' ' ')")
        continue
    fi
    # shellcheck disable=SC2086
    if ! "$LINK" "${argv[@]}" $EXTRA -o "$WORK/u/p" >"$WORK/link.log" 2>&1; then
        FAILED+=("$src — compiler/link failed unstripped: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
        continue
    fi
    if check_tools "$WORK/ld/p" "$WORK/u/p" "$src"; then
        compared=$((compared + 1))
    fi
done

# The C fixtures.
fx=0
for entry in "${LINK_FIXTURES[@]}"; do
    name=${entry%%|*}
    if ! link_fixture_build "$name" >"$WORK/fx.log" 2>&1; then
        FAILED+=("fixture $name — cc does not build it: $(head -2 "$WORK/fx.log" | tr '\n' ' ')")
        continue
    fi
    mkdir -p "$WORK/fx/$name/o"
    # The same name as cc's output, for the signature's identifier.
    # shellcheck disable=SC2086
    if ! "$LINK" "${FX_OBJS[@]}" "${FX_FLAGS[@]}" $EXTRA -o "$WORK/fx/$name/o/ref" >"$WORK/link.log" 2>&1; then
        FAILED+=("fixture $name — compiler/link failed: $(head -2 "$WORK/link.log" | tr '\n' ' ')")
        continue
    fi
    if check_tools "$WORK/fx/$name/ref" "$WORK/fx/$name/o/ref" "fixture $name"; then
        fx=$((fx + 1))
    fi
done

echo
echo "sampled: $sampled   byte-identical to the shipped build: $identical   tools agree: $compared   C fixtures agree: $fx of ${#LINK_FIXTURES[@]}   (ld $LD_VERSION)"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
    exit 1
fi
if [ "$identical" -ne "$sampled" ] || [ "$compared" -ne "$sampled" ] || [ "$fx" -ne ${#LINK_FIXTURES[@]} ]; then
    echo "not every sampled program was compared"
    exit 1
fi
# A quarter of the programs with a main, at least.
if [ "$sampled" -lt $(( considered / 4 )) ] || [ "$sampled" -lt 1 ]; then
    echo "the sample is too small: $sampled of $considered"
    exit 1
fi
exit 0
