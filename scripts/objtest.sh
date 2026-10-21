#!/usr/bin/env bash
# The Grasm backend's object files are objects the tools accept, and the
# same TIR makes the same bytes (docs/self-hosting.md, stage S3).
#
# For every corpus program in the subset (`scripts/s3_subset.py`), the object
# `compiler/s3` writes must:
#
#   * be bit-identical when compiled a second time, into another path;
#   * be read by `otool -l`, with the load commands this writer emits --
#     LC_SEGMENT_64 holding __text, LC_BUILD_VERSION, LC_SYMTAB, LC_DYSYMTAB;
#   * be read by `nm`, defining `_main` as an external;
#   * carry a relocation on every instruction the linker must complete -- a
#     BRANCH26 on each `bl`, a PAGE21 on each `adrp`, a PAGEOFF12 on each
#     page-offset `add` -- and none anywhere else (`scripts/s3_macho.py`,
#     which reads the file itself rather than a tool's presentation of it);
#   * link with `cc` and the runtime, with nothing on the linker's output:
#     the Cranelift build's objects make `ld` warn that they name no platform,
#     and this writer's must not.
#
# Not run: `dwarfdump --verify`. This stage emits no DWARF yet (line tables
# are S3 part two), and verifying an object with no debug information would
# pass while checking nothing.
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
s3_runtime
s3_collect

checked=0
identical=0
problems=0
declare -a FAILED=()
mkdir -p "$WORK/a" "$WORK/b/deeper"

for k in "${!S3_SRC[@]}"; do
    [ "${S3_RULE[$k]}" = "in" ] || continue
    src=${S3_SRC[$k]}
    tir=${S3_TIR[$k]}
    checked=$((checked + 1))
    a="$WORK/a/p.o"
    b="$WORK/b/deeper/q.o"
    rm -f "$a" "$b"
    if ! "$S3" "$tir" -o "$a" >"$WORK/s3.log" 2>&1 || ! "$S3" "$tir" -o "$b" >>"$WORK/s3.log" 2>&1; then
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
    if ! python3 scripts/s3_macho.py relocs "$a" >"$WORK/relocs.txt" 2>&1; then
        bad="$bad; relocations: $(head -2 "$WORK/relocs.txt" | tr '\n' ' ')"
    fi
    if s3_link "$a" "$WORK/prog" "$WORK/link.txt"; then
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

echo "objects: $checked   reproducible: $identical   with problems: $problems"
if [ ${#FAILED[@]} -gt 0 ]; then
    echo
    echo "FAILED:"
    printf '  %s\n' "${FAILED[@]}"
fi
[ "$problems" -eq 0 ] || exit 1
[ "$identical" -eq "$checked" ] || exit 1
if [ -z "${DIRS:-}" ] && [ "$checked" -lt "$S3_SUBSET_FLOOR" ]; then
    echo "only $checked objects were checked; the subset's floor is $S3_SUBSET_FLOOR"
    exit 1
fi
[ "$checked" -gt 0 ] || { echo "nothing was checked"; exit 1; }
exit 0
