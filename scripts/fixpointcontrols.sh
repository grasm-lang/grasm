#!/usr/bin/env bash
# The S6 gates must fail on a planted fault, each for its reason
# (docs/self-hosting.md §6.14).
#
# A control copies the compiler's sources -- every directory the TIR of its
# programs names: compiler/, tools/tirinterp and the packages they use --
# into a tree of its own, changes one thing there, and runs
# `scripts/fixpoint.sh` on that tree (FIXPOINT_ROOT). The gate must fail, and
# with the line the control names: a gate failing for another reason has not
# shown it sees this one.
#
#   timestamp        s3 writes the clock into every object, in its line
#                    table's producer                      NONDETERMINISTIC
#   hash-order       s3 writes a program's functions in the order of a hash
#                    seeded from `random_bytes`, as a hash table seeded per
#                    process iterates                      NONDETERMINISTIC
#   self-miscompile  s3, compiling the function `file_line` of a program's
#                    main module, makes each 1 that is added 2. Only the
#                    compiler has that function -- s3's own, which finds a
#                    line for the line table -- so stage 1, Cranelift's
#                    build, is right and compiles every corpus program it is
#                    tried on to the bytes it would without the fault, while
#                    stage 2's s3 writes line tables one line off: the
#                    executables of stage 2 and 3 are the same bytes and
#                    their objects are not             NOT A FIXED POINT
#                    and `scripts/stagediff.sh`, stage 1 against stage 2,
#                    sees every object differ
#   skip             the gate leaves its last comparison out
#                    (FIXPOINT_FAULT=skip)                 the count
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
GRASM=$(cd "$(dirname "$GRASM")" && pwd)/$(basename "$GRASM")
export GRASM
. "$(dirname "$0")/lib/stages.sh"
. "$(dirname "$0")/lib/corpus.sh"
corpus_require
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

caught=0
missed=0
ran=0
declare -a MISSED=()

# The directories the compiler's sources are in, from the TIR of each of its
# programs: a package's directory is the first two parts of a source's path.
: >"$WORK/dirs"
for e in "rt|compiler/rt/src/main.gr" "${STAGE_EXES[@]}"; do
    if ! "$GRASM" tir "${e#*|}" >"$WORK/list.tir" 2>"$WORK/list.err"; then
        echo "error: grasm tir ${e#*|} fails: $(head -1 "$WORK/list.err")"
        exit 1
    fi
    grep '^(file ' "$WORK/list.tir" | awk '{ print $4 }' | tr -d '"' | awk -F/ '{ print $1 "/" $2 }' >>"$WORK/dirs"
done
sort -u "$WORK/dirs" -o "$WORK/dirs"
if [ "$(grep -c . "$WORK/dirs")" -lt 8 ] || ! grep -qx compiler/s3 "$WORK/dirs"; then
    echo "error: the compiler's sources are in $(grep -c . "$WORK/dirs") directories, which cannot be all of them"
    exit 1
fi

# plant NAME FILE OLD NEW [OLD NEW...]: the compiler's sources copied to
# $WORK/NAME, with each OLD in FILE (a path in the tree) replaced by NEW,
# each exactly once.
plant() {
    local name=$1 file=$2 d
    shift 2
    rm -rf "${WORK:?}/$name"
    while IFS= read -r d; do
        mkdir -p "$WORK/$name/$(dirname "$d")"
        cp -R "$d" "$WORK/$name/$d" || return 1
    done <"$WORK/dirs"
    python3 - "$WORK/$name/$file" "$@" <<'EOF'
import sys
path = sys.argv[1]
pairs = sys.argv[2:]
text = open(path).read()
for old, new in zip(pairs[::2], pairs[1::2]):
    if text.count(old) != 1:
        print(f"the control's text occurs {text.count(old)} times in {path}: {old!r}")
        sys.exit(1)
    text = text.replace(old, new)
open(path, "w").write(text)
EOF
}

# verdict NAME WHY OUT: whether fixpoint's output OUT failed for WHY.
verdict() {
    local name=$1 why=$2 out=$3 rc=$4
    if [ "$rc" = 0 ]; then
        missed=$((missed + 1))
        MISSED+=("$name: fixpoint passed")
        echo "MISSED: $name (fixpoint passed)"
        return 1
    fi
    if grep -E "$why" "$out" >/dev/null; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$out" | head -1 | sed 's/^ *//' | cut -c1-200)"
        return 0
    fi
    missed=$((missed + 1))
    MISSED+=("$name: fixpoint failed, but not for its reason: $(grep -m3 -E 'FAILED|error|NOT|NONDET|compared' "$out" | tr '\n' ' ' | cut -c1-300)")
    echo "MISSED: $name failed for another reason"
    return 1
}

# control NAME WHY FILE OLD NEW...: plant, then fixpoint on the planted tree
# must fail with a line matching WHY -- and, for a fault that is not
# nondeterminism, with no NONDETERMINISTIC line, or it was caught for
# another reason.
control() {
    local name=$1 why=$2 file=$3 rc
    shift 3
    ran=$((ran + 1))
    if ! plant "$name" "$file" "$@" >"$WORK/$name.plant" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("$name: could not be planted: $(head -2 "$WORK/$name.plant" | tr '\n' ' ')")
        echo "MISSED: $name could not be planted"
        return 1
    fi
    FIXPOINT_ROOT="$WORK/$name" FIXPOINT_KEEP="$WORK/$name.stages" GRASM_NO_BUILD=1 scripts/fixpoint.sh >"$WORK/$name.out" 2>&1
    rc=$?
    case "$why" in
    NONDET*) ;;
    *)
        if grep -c NONDETERMINISTIC "$WORK/$name.out" >/dev/null; then
            missed=$((missed + 1))
            MISSED+=("$name: fixpoint called it nondeterministic: $(grep -m1 NONDETERMINISTIC "$WORK/$name.out" | cut -c1-200)")
            echo "MISSED: $name was called nondeterminism"
            return 1
        fi
        ;;
    esac
    verdict "$name" "$why" "$WORK/$name.out" "$rc"
}

S3_MAIN=compiler/s3/src/main.gr

control timestamp 'NONDETERMINISTIC \(stage 1.s compiler, run twice\): stage 2b.s [a-z.]+\.o is not stage 2.s' "$S3_MAIN" \
    '    unit.producer = "grasm compiler/s3"' \
    '    unit.producer = "grasm compiler/s3 at {now_millis()}"'

control hash-order 'NONDETERMINISTIC \(stage 1.s compiler, run twice\): stage 2b.s [a-z.]+\.o is not stage 2.s' "$S3_MAIN" \
    '/// Compiles a TIR file into an object file. Answers the exit status.' \
    '/// PLANTED (scripts/fixpointcontrols.sh): a hash of `s` seeded by `seed`.
fn planted_hash(s: str, seed: i64) -> i64 {
    let b = s.to_bytes()
    var h = seed
    for i in 0..b.len() {
        h = (h * 31 + b.at(i)) % 1000000007
    }
    h
}

/// Compiles a TIR file into an object file. Answers the exit status.' \
    '    for f in p.fns {
        if x64 {
            break
        }' \
    '    let seed = match random_bytes(3) {
        Ok(rb) -> rb.at(0) * 65536 + rb.at(1) * 256 + rb.at(2)
        Err(_) -> 0
    }
    var order: [i64] = []
    for k in 0..p.fns.len() {
        order.push(k)
    }
    order.sort_by(|k| planted_hash(p.fns[k].sym, seed))
    for ok in order {
        let f = p.fns[ok]
        if x64 {
            break
        }'

if control self-miscompile 'NOT A FIXED POINT: stage 3.s [a-z.]+\.o is not stage 2.s' "$S3_MAIN" \
    '    let p = match mir.build(src, lib, inline_limit) {' \
    '    var p = match mir.build(src, lib, inline_limit) {' \
    '    if check {
        let problems = if x64 { x86_64.check_rules() } else { aarch64.check_rules() }' \
    '    // PLANTED (scripts/fixpointcontrols.sh): in `main::file_line`, a 1
    // that is added is made 2.
    for fi in 0..p.fns.len() {
        if p.fns[fi].name == "main::file_line" {
            var f = p.fns[fi]
            var one_at: Map[i64, i64] = {}
            for k in 0..f.op.len() {
                if f.op[k] == ir.M_ICONST and f.imm[k] == 1 {
                    one_at.set(f.res[k], k)
                }
            }
            for k in 0..f.op.len() {
                if f.op[k] == ir.M_ADD and one_at.has(f.b[k]) {
                    f.imm[one_at.get(f.b[k]) ?? 0] = 2
                }
            }
            p.fns[fi] = f
        }
    }
    if check {
        let problems = if x64 { x86_64.check_rules() } else { aarch64.check_rules() }'; then
    st="$WORK/self-miscompile.stages"
    # That the executables are the same bytes and only the objects differ:
    # what the fault does is the line table, which only an object holds.
    exe_same=0
    for e in "${STAGE_EXES[@]}"; do
        cmp -s "$st/2/${e%%|*}" "$st/3/${e%%|*}" && exe_same=$((exe_same + 1))
    done
    # Stage 1 with the fault is stage 1 without it on the corpus: every
    # tenth program with a main that the backend compiles gives the same
    # object from both.
    sample=0
    alike=0
    if "$GRASM" build "$S3_MAIN" --native -o "$WORK/s3.clean" >"$WORK/clean.log" 2>&1; then
        while IFS= read -r src; do
            grep -qE '^\s*(pub )?fn main\(' "$src" || continue
            grep -q '^//! expect: error$' "$src" && continue
            "$GRASM" tir "$src" >"$WORK/s.tir" 2>/dev/null || continue
            "$WORK/s3.clean" "$WORK/s.tir" -o "$WORK/s.clean.o" >/dev/null 2>&1 || continue
            sample=$((sample + 1))
            echo "$src" >>"$WORK/sample"
            "$st/1/s3" "$WORK/s.tir" -o "$WORK/s.planted.o" >/dev/null 2>&1 &&
                cmp -s "$WORK/s.clean.o" "$WORK/s.planted.o" && alike=$((alike + 1))
        done < <(corpus_programs | awk 'NR % 10 == 0')
    fi
    ran=$((ran + 1))
    if [ "$exe_same" = ${#STAGE_EXES[@]} ] && [ "$sample" -ge 20 ] && [ "$alike" = "$sample" ]; then
        caught=$((caught + 1))
        echo "caught: self-miscompile/stage-1 -- stage 1 with the fault compiles $alike of $sample sampled corpus programs to the same bytes as without it, and stage 3's $exe_same executables are stage 2's: only the objects show it"
    else
        missed=$((missed + 1))
        MISSED+=("self-miscompile/stage-1: executables alike $exe_same of ${#STAGE_EXES[@]}, corpus objects alike $alike of $sample")
        echo "MISSED: self-miscompile/stage-1"
    fi
    ran=$((ran + 1))
    head -20 "$WORK/sample" >"$WORK/sample20"
    if STAGE_A="$st/1" STAGE_B="$st/2" FILES="$WORK/sample20" GRASM_NO_BUILD=1 scripts/stagediff.sh >"$WORK/sd.out" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("self-miscompile/stagediff: stagediff passed")
        echo "MISSED: self-miscompile/stagediff (stagediff passed)"
    elif grep -E 'the runtime each compiler makes differs|object differs: the objects differ' "$WORK/sd.out" >/dev/null &&
        ! grep -E 'front tir:|executable differs' "$WORK/sd.out" >/dev/null; then
        caught=$((caught + 1))
        echo "caught: self-miscompile/stagediff -- $(grep -hE 'objects differ:' "$WORK/sd.out" | head -1 | sed 's/^ *//' | cut -c1-200)"
    else
        missed=$((missed + 1))
        MISSED+=("self-miscompile/stagediff: failed, but not for its reason: $(grep -m3 -E 'FAILED|differ|error' "$WORK/sd.out" | tr '\n' ' ' | cut -c1-300)")
        echo "MISSED: self-miscompile/stagediff failed for another reason"
    fi
fi

# No plant: the gate itself told to leave a comparison out.
ran=$((ran + 1))
FIXPOINT_FAULT=skip GRASM_NO_BUILD=1 scripts/fixpoint.sh >"$WORK/skip.out" 2>&1
verdict skip 'compared [0-9]+ of the [0-9]+ artifacts' "$WORK/skip.out" $?

echo
echo "controls: $ran   caught: $caught   missed: $missed"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
fi
[ "$missed" = 0 ] && [ "$caught" = "$ran" ] && [ "$ran" -ge 6 ]
