# What the linker gates share (docs/self-hosting.md, stage S4): the linker
# under test, and the objects and link line a native build hands its linker.
#
# Sourced after `corpus.sh`, with `GRASM` and `WORK` set.

# The linker under test, in `$LINK`. Built here from `compiler/link`, natively
# and once per gate, so no gate passes against a linker older than its
# source. A caller that names its own binary through `LINK=` gets that one.
link_driver() {
    if [ -n "${LINK:-}" ]; then
        if [ ! -x "$LINK" ]; then
            echo "error: LINK=$LINK is not an executable"
            exit 1
        fi
        return 0
    fi
    LINK="$WORK/link"
    if ! "$GRASM" build compiler/link/src/main.gr --native -o "$LINK" >"$WORK/link-build.log" 2>&1; then
        echo "error: compiler/link does not build:"
        sed 's/^/  /' "$WORK/link-build.log"
        exit 1
    fi
}

# Builds `$1` natively the way `grasm build --native` does with `cc`
# (GRASM_LINKER=cc), into `$2`, keeping the objects it linked and its link
# line in `$2.cap/` (`scripts/lib/linkcc/cc`). Fails when the build does.
link_capture() {
    local src=$1 out=$2
    rm -rf "$out.cap" "$out"
    PATH="$PWD/scripts/lib/linkcc:$PATH" LINKCC_DIR="$out.cap" GRASM_LINKER=cc \
        "$GRASM" build "$src" --native -o "$out" >"$out.log" 2>&1 && [ -x "$out" ] && [ -f "$out.cap/args" ]
}

# The arguments for `$LINK` that reproduce a captured link line `$1/args`:
# the objects, the runtime, the library flags as given, -dead_strip, and each
# `-exported_symbol`. One argument per line on stdout.
link_args() {
    local cap=$1 kind val
    while read -r kind val; do
        [ "$kind" = OBJ ] && echo "$cap/$val"
    done <"$cap/args"
    local skip_next=0
    while read -r kind val; do
        [ "$kind" = ARG ] || continue
        if [ "$skip_next" = 1 ]; then
            skip_next=0
            continue
        fi
        case "$val" in
        -o) skip_next=1 ;;
        -Wl,-dead_strip) echo "-dead_strip" ;;
        -Wl,-exported_symbol,*)
            echo "-exported_symbol"
            echo "${val#-Wl,-exported_symbol,}"
            ;;
        -Wl,*)
            echo "unsupported linker flag in the capture: $val" >&2
            return 1
            ;;
        *) echo "$val" ;;
        esac
    done <"$cap/args"
}

# The C fixtures (`scripts/fixtures/link/`): what neither backend emits but a
# Mach-O linker must get right. Each is `name|sources|exported symbols`;
# `link_fixture_build NAME` compiles one with `cc -c` into $WORK/fx/NAME/,
# links the reference there with cc (`ref`), and sets FX_OBJS and FX_FLAGS to
# the objects and the flags compiler/link takes for the same link.
LINK_FIXTURES=(
    "tlv|tlv_a.c tlv_b.c|"
    "export|export.c|_only_by_name"
)

link_fixture_build() {
    local want=$1 entry name srcs exports s
    for entry in "${LINK_FIXTURES[@]}"; do
        IFS='|' read -r name srcs exports <<<"$entry"
        [ "$name" = "$want" ] || continue
        local dir="$WORK/fx/$name"
        rm -rf "$dir"
        mkdir -p "$dir"
        FX_OBJS=()
        FX_FLAGS=(-dead_strip)
        local ccflags=(-Wl,-dead_strip)
        for s in $srcs; do
            cc -c "scripts/fixtures/link/$s" -o "$dir/${s%.c}.o" || return 1
            FX_OBJS+=("$dir/${s%.c}.o")
        done
        for s in $exports; do
            FX_FLAGS+=(-exported_symbol "$s")
            ccflags+=("-Wl,-exported_symbol,$s")
        done
        cc "${FX_OBJS[@]}" -o "$dir/ref" "${ccflags[@]}"
        return $?
    done
    echo "no fixture named $want"
    return 1
}

# Whether SDL2 can be found where `cc` would find it, with LIBRARY_PATH set
# as `scripts/sdltest.sh` sets it; answers on stdout.
link_sdl_present() {
    if [ -z "${LIBRARY_PATH:-}" ] && command -v brew >/dev/null 2>&1; then
        LIBRARY_PATH="$(brew --prefix)/lib"
        export LIBRARY_PATH
    fi
    local d
    for d in ${LIBRARY_PATH:-} /usr/local/lib; do
        if [ -e "$d/libSDL2.dylib" ]; then
            echo yes
            return 0
        fi
    done
    echo no
}
