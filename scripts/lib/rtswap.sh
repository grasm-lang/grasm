# What the S5 gates share (docs/self-hosting.md §6.7, §6.12): the runtime
# written in Grasm built to an object -- the ordinary build and the poisoned
# one -- held to its list of families, and a linker for `grasm build
# --native` that links every program twice: as it ships, with the Rust
# runtime archive, and with the Grasm runtime alone in its place.
#
# Sourced after `corpus.sh`, `fresh.sh`, `s3.sh` and `link.sh`, with `GRASM`
# and `WORK` set. `RT_OBJ=<object>` names a runtime object instead of
# building one (the controls plant faults that way), and `RT_OBJ_POISON` its
# poisoned build, `<RT_OBJ less .o>.poison.o` when not given.

RT_SRC=compiler/rt/src/main.gr
RT_FAMILIES=compiler/rt/families.txt

# Builds the runtime whose package is the directory `$1` (its `src/main.gr`)
# into object `$2` with the Grasm backend (`$S3`): `grasm tir` (or `$FRONT
# tir`, when `FRONT=` names a frontend), then `s3 --runtime`. With `$3` =
# poison, from a copy of it whose `build.gr` says so. Answers whether it
# built; why not goes to `$2.log`.
rtswap_build() {
    local dir=$1 obj=$2 flavour=${3:-}
    if [ "$flavour" = poison ]; then
        rm -rf "$obj.src"
        cp -R "$dir" "$obj.src"
        if [ "$(grep -c '^pub let POISON = false$' "$obj.src/src/build.gr")" != 1 ]; then
            echo "compiler/rt/src/build.gr does not say \`pub let POISON = false\` once" >"$obj.log"
            return 1
        fi
        sed -i '' 's/^pub let POISON = false$/pub let POISON = true/' "$obj.src/src/build.gr"
        dir="$obj.src"
    fi
    "${FRONT:-$GRASM}" tir "$dir/src/main.gr" >"$obj.tir" 2>"$obj.log" &&
        "$S3" "$obj.tir" -o "$obj" --runtime >>"$obj.log" 2>&1
}

# Builds the Grasm runtime into `$RT_OBJ` and its poisoned build into
# `$RT_OBJ_POISON`. Stops the gate when either does not build.
rtswap_runtime() {
    if [ -n "${RT_OBJ:-}" ]; then
        if [ ! -f "$RT_OBJ" ]; then
            echo "error: RT_OBJ=$RT_OBJ is missing"
            exit 1
        fi
        RT_OBJ_POISON=${RT_OBJ_POISON:-${RT_OBJ%.o}.poison.o}
        if [ ! -f "$RT_OBJ_POISON" ]; then
            echo "error: RT_OBJ_POISON=$RT_OBJ_POISON is missing"
            exit 1
        fi
        return 0
    fi
    RT_OBJ="$WORK/rt.o"
    RT_OBJ_POISON="$WORK/rt.poison.o"
    if ! rtswap_build compiler/rt "$RT_OBJ"; then
        echo "error: the Grasm runtime does not build:"
        sed 's/^/  /' "$RT_OBJ.log" | head -20
        exit 1
    fi
    if ! rtswap_build compiler/rt "$RT_OBJ_POISON" poison; then
        echo "error: the Grasm runtime's poisoned build does not build:"
        sed 's/^/  /' "$RT_OBJ_POISON.log" | head -20
        exit 1
    fi
}

# Holds the runtime object to `families.txt`: every moved name exported as
# `_grt_<name>`, nothing exported that is not listed, and no `_grasm_` name
# called -- the Grasm runtime is linked alone, with nothing else to reach.
# Writes the moved names to `$WORK/moved.txt` and says how many of each.
rtswap_families() {
    local exported moved called bad=0 n
    exported=$(nm -g "$RT_OBJ" | awk '$2 == "T" && $3 ~ /^_grt_/ { sub(/^_grt_/, "", $3); print $3 }' | LC_ALL=C sort -u)
    moved=$(awk '!/^#/ && NF >= 2 { print $2 }' "$RT_FAMILIES" | LC_ALL=C sort -u)
    called=$(nm -u "$RT_OBJ" | awk '$1 ~ /^_grasm_/ { sub(/^_grasm_/, "", $1); print $1 }' | LC_ALL=C sort -u)
    for n in $(LC_ALL=C comm -23 <(echo "$moved") <(echo "$exported")); do
        echo "FAIL: families.txt says $n has moved, but the Grasm runtime does not export _grt_$n"
        bad=1
    done
    for n in $(LC_ALL=C comm -13 <(echo "$moved") <(echo "$exported")); do
        echo "FAIL: the Grasm runtime exports _grt_$n, which families.txt does not list"
        bad=1
    done
    for n in $called; do
        echo "FAIL: the Grasm runtime calls _grasm_$n, which nothing linked with it defines"
        bad=1
    done
    if [ "$bad" != 0 ]; then
        exit 1
    fi
    echo "$moved" >"$WORK/moved.txt"
    RT_MOVED=$(grep -c . "$WORK/moved.txt")
    echo "the Grasm runtime: $RT_MOVED names moved ($(awk '!/^#/ && NF >= 2 { print $1 }' "$RT_FAMILIES" | sort | uniq -c | awk '{ printf "%s%s %s", sep, $2, $1; sep = ", " }')); it calls none of the Rust runtime's"
}

# Points `grasm build --native` at `scripts/lib/rtswap_link`.
rtswap_linker() {
    export RTSWAP_LINK="$LINK"
    export RTSWAP_RT="$RT_OBJ"
    export RTSWAP_RT_POISON="$RT_OBJ_POISON"
    export GRASM_LINKER="$PWD/scripts/lib/rtswap_link"
}
