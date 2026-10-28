# What the S5 gates share (docs/self-hosting.md §6.7): the runtime written in
# Grasm built to an object, held to its list of families, and a linker for
# `grasm build --native` that links every program twice, with and without
# it in front of the Rust runtime.
#
# Sourced after `corpus.sh`, `fresh.sh`, `s3.sh` and `link.sh`, with `GRASM`
# and `WORK` set. `RT_OBJ=<object>` names a runtime object instead of
# building one (the controls plant faults that way).

RT_SRC=compiler/rt/src/main.gr
RT_FAMILIES=compiler/rt/families.txt

# Builds the Grasm runtime into `$RT_OBJ` with the Grasm backend (`$S3`):
# `grasm tir`, then `s3 --runtime`. Stops the gate when it does not build.
rtswap_runtime() {
    if [ -n "${RT_OBJ:-}" ]; then
        if [ ! -f "$RT_OBJ" ]; then
            echo "error: RT_OBJ=$RT_OBJ is missing"
            exit 1
        fi
        return 0
    fi
    RT_OBJ="$WORK/rt.o"
    if ! "$GRASM" tir "$RT_SRC" >"$WORK/rt.tir" 2>"$WORK/rt-tir.log"; then
        echo "error: the Grasm runtime does not emit TIR:"
        sed 's/^/  /' "$WORK/rt-tir.log" | head -20
        exit 1
    fi
    if ! "$S3" "$WORK/rt.tir" -o "$RT_OBJ" --runtime >"$WORK/rt-s3.log" 2>&1; then
        echo "error: the Grasm runtime does not compile:"
        sed 's/^/  /' "$WORK/rt-s3.log" | head -20
        exit 1
    fi
}

# Holds the runtime object to `families.txt`: every moved name exported as
# `_grt_<name>` and nothing exported that is not listed, and every
# `_grasm_<name>` it calls listed as state or as the backend's own. Writes the
# moved names to `$WORK/moved.txt` and says how many of each.
rtswap_families() {
    local exported moved called allowed bad=0 n
    exported=$(nm -g "$RT_OBJ" | awk '$2 == "T" && $3 ~ /^_grt_/ { sub(/^_grt_/, "", $3); print $3 }' | LC_ALL=C sort -u)
    moved=$(awk '!/^#/ && NF >= 2 && $1 != "state" && $1 != "backend" && $1 != "uses" { print $2 }' "$RT_FAMILIES" | LC_ALL=C sort -u)
    allowed=$(awk '!/^#/ && ($1 == "state" || $1 == "backend" || $1 == "uses") { print $2 }' "$RT_FAMILIES" | LC_ALL=C sort -u)
    called=$(nm -u "$RT_OBJ" | awk '$1 ~ /^_grasm_/ { sub(/^_grasm_/, "", $1); print $1 }' | LC_ALL=C sort -u)
    for n in $(LC_ALL=C comm -23 <(echo "$moved") <(echo "$exported")); do
        echo "FAIL: families.txt says $n has moved, but the Grasm runtime does not export _grt_$n"
        bad=1
    done
    for n in $(LC_ALL=C comm -13 <(echo "$moved") <(echo "$exported")); do
        echo "FAIL: the Grasm runtime exports _grt_$n, which families.txt does not list"
        bad=1
    done
    for n in $(LC_ALL=C comm -23 <(echo "$called") <(echo "$allowed")); do
        echo "FAIL: the Grasm runtime calls the Rust runtime's _grasm_$n, which families.txt allows neither as state, as the backend's nor as a use"
        bad=1
    done
    if [ "$bad" != 0 ]; then
        exit 1
    fi
    echo "$moved" >"$WORK/moved.txt"
    RT_MOVED=$(grep -c . "$WORK/moved.txt")
    RT_CALLS=$(echo "$called" | grep -c .)
    echo "the Grasm runtime: $RT_MOVED names moved ($(awk '!/^#/ && NF >= 2 && $1 != "state" && $1 != "backend" && $1 != "uses" { print $1 }' "$RT_FAMILIES" | sort | uniq -c | awk '{ printf "%s%s %s", sep, $2, $1; sep = ", " }')); it calls $RT_CALLS of the Rust runtime's ($(echo "$called" | tr '\n' ' ' | sed 's/ $//'))"
}

# Points `grasm build --native` at `scripts/lib/rtswap_link`.
rtswap_linker() {
    export RTSWAP_LINK="$LINK"
    export RTSWAP_RT="$RT_OBJ"
    export GRASM_LINKER="$PWD/scripts/lib/rtswap_link"
}
