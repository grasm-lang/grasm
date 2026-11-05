# What the Linux gates share (docs/self-hosting.md §6.18, stage S7): the
# machine Linux runs in, and the x86-64 C library and linker for it.
#
# Linux runs in an OrbStack machine on this Mac (the owner's decision,
# 2026-11-03), `ubuntu` unless `GRASM_LINUX_MACHINE` names another: arm64,
# with OrbStack's Rosetta for x86-64. Everything is built on the Mac and run
# in the machine from where it lies, through OrbStack's mount of the Mac's
# `/Users` and `/private`, so nothing is written to the machine's own
# filesystem; a gate's work directory is `mktemp -d` resolved through
# `/private`, which the machine sees at the same path.
#
# A gate that needs the machine and cannot reach it does not run: it says
# why on its last line and exits 77, which `allgates.sh` counts as not run.
# The machine is never started or stopped by a gate -- it must be running --
# so OrbStack is left as it was found.
#
# The x86-64 half needs an x86-64 glibc to run on and an x86-64 GNU ld to
# link the reference with, which the machine does not have. They are
# Ubuntu's arm64 cross packages, unpacked into a directory on the Mac and
# not installed in the machine (the owner's decision, 2026-11-04), named by
# `GRASM_X64_ROOT`; `linux_x64_root_make <dir>` makes one.

LINUX_MACHINE=${GRASM_LINUX_MACHINE:-ubuntu}
LINUX_NOT_RUN=77
LINUX_TIMEOUT=""
command -v timeout >/dev/null 2>&1 && LINUX_TIMEOUT="timeout 60"

# Stops the gate as not run when the machine cannot be reached.
linux_require() {
    if ! command -v orb >/dev/null 2>&1; then
        echo "not run: OrbStack's \`orb\` is not installed, and the Linux gates run in its machine \`$LINUX_MACHINE\` (docs/self-hosting.md §6.18)"
        exit "$LINUX_NOT_RUN"
    fi
    if ! orb list 2>/dev/null | awk -v m="$LINUX_MACHINE" '$1 == m && $2 == "running" { found = 1 } END { exit !found }'; then
        echo "not run: the OrbStack machine \`$LINUX_MACHINE\` is not running (a gate never starts it; docs/self-hosting.md §6.18)"
        exit "$LINUX_NOT_RUN"
    fi
    # Bounded: once, a first `orb` call hung for twenty minutes and answered
    # at once when tried again.
    local try
    for try in 1 2 3; do
        if $LINUX_TIMEOUT orb -m "$LINUX_MACHINE" true >/dev/null 2>&1; then
            return 0
        fi
    done
    echo "not run: the OrbStack machine \`$LINUX_MACHINE\` does not answer"
    exit "$LINUX_NOT_RUN"
}

# Runs a command in the machine.
linux_run() {
    orb -m "$LINUX_MACHINE" "$@"
}

# A work directory both the Mac and the machine see at the same path.
linux_workdir() {
    local d
    d=$(mktemp -d)
    (cd "$d" && pwd -P)
}

# Stops the gate as not run when the x86-64 root is missing; otherwise sets
# `X64_ROOT`, `X64_LIB` (its C library's directory, as the machine sees it
# under the root), `X64_LD` and `X64_RUN` (scripts that run, in the machine,
# its ld with the root as the system root, and a program through its
# dynamic loader).
linux_x64_root() {
    X64_ROOT=${GRASM_X64_ROOT:-}
    if [ -z "$X64_ROOT" ] || [ ! -x "$X64_ROOT/usr/x86_64-linux-gnu/lib/ld-linux-x86-64.so.2" ] ||
        [ ! -x "$X64_ROOT/usr/bin/x86_64-linux-gnu-ld" ]; then
        echo "not run: GRASM_X64_ROOT does not name the unpacked x86-64 cross packages (make one with \`linux_x64_root_make <dir>\` from scripts/lib/linux.sh; docs/self-hosting.md §6.18)"
        exit "$LINUX_NOT_RUN"
    fi
    X64_ROOT=$(cd "$X64_ROOT" && pwd -P)
    X64_LIB=/usr/x86_64-linux-gnu/lib
    X64_LD="$WORK/x64-ld"
    X64_RUN="$WORK/x64-run"
    cat >"$X64_LD" <<EOF
#!/bin/sh
LD_LIBRARY_PATH=$X64_ROOT/usr/lib/aarch64-linux-gnu exec $X64_ROOT/usr/bin/x86_64-linux-gnu-ld --sysroot=$X64_ROOT "\$@"
EOF
    cat >"$X64_RUN" <<EOF
#!/bin/sh
exec $X64_ROOT$X64_LIB/ld-linux-x86-64.so.2 --library-path $X64_ROOT$X64_LIB "\$@"
EOF
    chmod +x "$X64_LD" "$X64_RUN"
}

# The packages `GRASM_X64_ROOT` holds, fetched by the machine's apt from its
# own sources (no root needed, nothing installed) and unpacked into `$1`.
LINUX_X64_PACKAGES="libc6-amd64-cross libc6-dev-amd64-cross linux-libc-dev-amd64-cross libgcc-s1-amd64-cross binutils-x86-64-linux-gnu"
linux_x64_root_make() {
    local dir
    mkdir -p "$1/debs" || return 1
    dir=$(cd "$1" && pwd -P)
    # shellcheck disable=SC2086
    linux_run sh -c "cd '$dir/debs' && apt-get download $LINUX_X64_PACKAGES && for d in *.deb; do dpkg-deb -x \"\$d\" ..; done"
}

# The Grasm runtime for Linux target `$1` (`aarch64-linux`, `x86_64-linux`)
# built into object `$2` with the Grasm backend (`$S3`): `compiler/rt` copied
# with `os/linux.gr` in `src/os.gr`'s place (docs/self-hosting.md §6.18), its
# TIR by `grasm tir` (or `$FRONT tir`), then `s3 --runtime --target $1`.
# With `$3` = poison, the poisoned build, its `build.gr` changed as
# `rtswap.sh` changes it. `ELF_RT_SRC=` names another copy of `compiler/rt`
# (the controls plant faults in one). Answers whether it built; why not goes to `$2.log`.
linux_rt_build() {
    local t=$1 obj=$2 flavour=${3:-} dir="$2.src"
    rm -rf "$dir"
    mkdir -p "$dir"
    cp -R "${ELF_RT_SRC:-compiler/rt}" "$dir/rt"
    cp "${ELF_RT_SRC:-compiler/rt}/os/linux.gr" "$dir/rt/src/os.gr"
    if [ "$flavour" = poison ]; then
        if [ "$(grep -c '^pub let POISON = false$' "$dir/rt/src/build.gr")" != 1 ]; then
            echo "compiler/rt/src/build.gr does not say \`pub let POISON = false\` once" >"$obj.log"
            return 1
        fi
        sed -i '' 's/^pub let POISON = false$/pub let POISON = true/' "$dir/rt/src/build.gr"
    fi
    "${FRONT:-$GRASM}" tir "$dir/rt/src/main.gr" >"$obj.tir" 2>"$obj.log" &&
        "$S3" "$obj.tir" -o "$obj" --runtime --target "$t" >>"$obj.log" 2>&1
}

# GNU ld's `--defsym` for each runtime name the Grasm runtime object `$1`
# defines, binding a program's `grasm_<name>` to its `grt_<name>`, as
# `compiler/link -interpose` binds them, one a line into `$2`.
linux_defsyms() {
    "${LLVM_NM:-/opt/homebrew/opt/llvm@21/bin/llvm-nm}" -g --defined-only "$1" |
        awk '$3 ~ /^grt_/ { n = $3; sub(/^grt_/, "", n); printf "--defsym=grasm_%s=grt_%s\n", n, n }' >"$2"
    [ -s "$2" ]
}

# How a gate runs a corpus program in the machine: with a /tmp of its own,
# a directory of the gate's (`scripts/lib/isolate.py`, given the directory
# and the program), so that what a program writes under /tmp -- fifteen of
# the subset's do -- lands in the gate's work directory and not in the
# machine's /tmp, which is the owner's, and two runs never share it. The
# program runs as the machine's user, with that user's permissions.
# Checked by `linux_isolate_check`.
LINUX_ISOLATE="python3 $PWD/scripts/lib/isolate.py"
linux_isolate_check() {
    mkdir -p "$WORK/isolate"
    if ! linux_run sh -c "$LINUX_ISOLATE '$WORK/isolate' sh -c 'echo probe >/tmp/grasm-isolate-probe'" ||
        [ ! -f "$WORK/isolate/grasm-isolate-probe" ]; then
        echo "error: the machine cannot run a program with a /tmp of its own (scripts/lib/isolate.py)"
        exit 1
    fi
    if linux_run test -e /tmp/grasm-isolate-probe; then
        echo "error: a program run with a /tmp of its own wrote into the machine's /tmp"
        exit 1
    fi
}

# Keeps, of what `s3_collect` found, only the programs whose path matches
# the regular expression `ELF_PROGRAMS`, when it is set (the controls run a
# few programs that reach what they break). A gate run so filtered is a
# partial one and holds no floor.
linux_filter_programs() {
    [ -n "${ELF_PROGRAMS:-}" ] || return 0
    local -a src=() tir=() rule=()
    local k
    for k in "${!S3_SRC[@]}"; do
        if [[ ${S3_SRC[$k]} =~ $ELF_PROGRAMS ]]; then
            src+=("${S3_SRC[$k]}")
            tir+=("${S3_TIR[$k]}")
            rule+=("${S3_RULE[$k]}")
        fi
    done
    S3_SRC=("${src[@]}")
    S3_TIR=("${tir[@]}")
    S3_RULE=("${rule[@]}")
    DIRS=${DIRS:-filtered}
}
