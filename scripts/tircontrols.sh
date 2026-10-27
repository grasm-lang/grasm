#!/usr/bin/env bash
# The S2 part two gates must fail on a planted fault (docs/self-hosting.md,
# S2 part two).
#
# Each control copies `compiler/front`, changes one thing in the copy, builds
# it, and runs the gate that should catch it against it (`FRONT=`), on files
# that reach the fault. The gate must fail, and for the reason the control
# names -- a gate failing for some other reason has not shown it sees this.
#
#   wrong-module     a package's import of one of its dependencies
#                    resolved to the top-level module of that name       tirdiff
#   widened          an integer literal checked against `u8` typed `i64` tirdiff
#   generic-twice    a call's type arguments made apart from the ones
#                    its arguments were checked against                  tirdiff
#   effect-dropped   a called function's last effect left out of a row   tirdiff
#   refusal-accepted a type mismatch accepted                            tirrefuse
#   field-swapped    `index` printed with its span before its type       tirdiff
#   toml-entries     packages/toml swaps two keys' values between the
#                    entries of an array of tables                       tomldiff
#   fuzz-skips       the fuzzer skips one of its engines                 fuzz_tir
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
export GRASM
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

caught=0
missed=0
ran=0
declare -a MISSED=()

# Files that reach the faults: the conformance programs, the examples, and
# two packages whose dependencies import packages of their own.
git ls-files 'spec/conformance/*.gr' 'examples/*.gr' >"$WORK/files"
echo packages/accounts-demo/src/main.gr >>"$WORK/files"
echo packages/b35-demo/src/main.gr >>"$WORK/files"
# For tomldiff: the files with arrays of tables, and a manifest.
git ls-files 'scripts/lib/tomlcases/*.toml' >"$WORK/files.tomldiff"
echo compiler/front/grasm.toml >>"$WORK/files.tomldiff"

# plant NAME FILE OLD NEW [OLD NEW...]: a copy of compiler/front (beside the
# packages/toml it depends on) with each OLD replaced by NEW (each exactly
# once), built to $WORK/NAME. FILE is in compiler/front/src, or a path from
# the checkout's root.
plant() {
    local name=$1 file=$2
    shift 2
    rm -rf "${WORK:?}/$name.src"
    mkdir -p "$WORK/$name.src/compiler" "$WORK/$name.src/packages"
    cp -R compiler/front "$WORK/$name.src/compiler/front"
    cp -R packages/toml "$WORK/$name.src/packages/toml"
    case $file in
        */*) file="$WORK/$name.src/$file" ;;
        *) file="$WORK/$name.src/compiler/front/src/$file" ;;
    esac
    if ! python3 - "$file" "$@" <<'EOF'; then
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
        return 1
    fi
    "$GRASM" build "$WORK/$name.src/compiler/front/src/main.gr" --native -o "$WORK/$name" >"$WORK/$name.log" 2>&1
}

# expect NAME WHY GATE...: the gate, run against the planted build, fails
# with output matching WHY. A gate with its own list ($WORK/files.GATE) is
# given that one.
expect() {
    local name=$1 why=$2
    shift 2
    ran=$((ran + 1))
    local files="$WORK/files.$(basename "$1" .sh)"
    [ -f "$files" ] || files="$WORK/files"
    if FRONT="$WORK/$name" FILES="$files" GRASM_NO_BUILD=1 "$@" >"$WORK/$name.out" 2>&1; then
        missed=$((missed + 1))
        MISSED+=("$name: $* passed")
        echo "MISSED: $name ($*)"
    elif grep -qE "$why" "$WORK/$name.out"; then
        caught=$((caught + 1))
        echo "caught: $name -- $(grep -hE "$why" "$WORK/$name.out" | head -1 | cut -c1-160)"
    else
        missed=$((missed + 1))
        MISSED+=("$name: $* failed, but not for its reason: $(tail -2 "$WORK/$name.out" | tr '\n' ' ' | cut -c1-200)")
        echo "MISSED: $name failed for another reason"
    fi
}

control() {
    local name=$1 why=$2 gate=$3 file=$4
    shift 4
    if ! plant "$name" "$file" "$@"; then
        ran=$((ran + 1))
        missed=$((missed + 1))
        MISSED+=("$name: could not be planted: $(tail -3 "$WORK/$name.log" 2>/dev/null | tr '\n' ' ')")
        echo "MISSED: $name could not be planted"
        return
    fi
    expect "$name" "$why" "$gate"
}

control wrong-module 'accepts it and front tir refuses it|TIR differs' scripts/tirdiff.sh loader.gr \
    '        None -> want
    }
}' \
    '        None -> written
    }
}'
control widened 'TIR differs' scripts/tirdiff.sh sema.gr \
    '        c.f_types.set(prog.ekey(p, e), want)
        return
    }
    if k == ast.E_UNARY and prog.sl(p, e, 0) == 0 and wk == tys.K_INT {' \
    '        c.f_types.set(prog.ekey(p, e), tys.I64)
        return
    }
    if k == ast.E_UNARY and prog.sl(p, e, 0) == 0 and wk == tys.K_INT {'
control generic-twice 'TIR differs' scripts/tirdiff.sh semcall.gr \
    'sema.tir_call(c, prog.key(s, e), sema.CR_FN, owner + "::" + name, fresh)' \
    'var fresh2: [i64] = []
    for _ in 0..c.fn_ngen[fid] {
        fresh2.push(tys.fresh(t))
    }
    sema.tir_call(c, prog.key(s, e), sema.CR_FN, owner + "::" + name, fresh2)'
control effect-dropped 'TIR differs' scripts/tirdiff.sh effrows.gr \
    '    for e in x.rows[f] {' \
    '    for e in x.rows[f].slice(0, if x.rows[f].len() > 0 { x.rows[f].len() - 1 } else { 0 }) {'
control refusal-accepted 'refuses it .* and the Grasm one accepts it' scripts/tirrefuse.sh sema.gr \
    'pub fn unify_or_report(inout c: Ck, inout t: Tys, s: i64, e: i64, got: i64, want: i64) {
    if tys.unify(t, got, want) {
        return
    }' \
    'pub fn unify_or_report(inout c: Ck, inout t: Tys, s: i64, e: i64, got: i64, want: i64) {
    if tys.unify(t, got, want) or true {
        return
    }'
control field-swapped 'TIR differs' scripts/tirdiff.sh emit.gr \
    'let h = head(c, t, m, "index", s, en)' \
    'let h = "index {sp(s, en)} " + expr_ty(c, t, m, prog.key(s, en))'

# Two keys of the same name in two entries of an array of tables, their
# values traded: every key is still there with a value it was given, so only
# a dump that says which entry each key is in can tell.
control toml-entries 'packages/toml and the Rust reader differ' scripts/tomldiff.sh packages/toml/src/main.gr \
    '    Doc{ keys: keys, values: values, entries: entries, ok: true, why: "", line: 0 }' \
    '    var traded = false
    for i in 0..keys.len() {
        let from = i + 1
        for j in from..keys.len() {
            if not traded and keys[i] == keys[j] and entries[i] != entries[j] {
                let v = values[i]
                values[i] = values[j]
                values[j] = v
                traded = true
            }
        }
    }
    Doc{ keys: keys, values: values, entries: entries, ok: true, why: "", line: 0 }'

# The fuzzer, told to skip an engine: its own counts must give it away.
ran=$((ran + 1))
if FUZZ_TIR_SKIP=generics FRONT="${FRONT_OK:-}" GRASM_NO_BUILD=1 python3 scripts/fuzz_tir.py --programs 140 >"$WORK/fuzz.out" 2>&1; then
    missed=$((missed + 1))
    MISSED+=("fuzz-skips: scripts/fuzz_tir.py passed while skipping")
    echo "MISSED: fuzz-skips"
elif grep -q 'an engine that skips is not a pass' "$WORK/fuzz.out"; then
    caught=$((caught + 1))
    echo "caught: fuzz-skips -- $(grep -h 'an engine that skips' "$WORK/fuzz.out" | head -1)"
else
    missed=$((missed + 1))
    MISSED+=("fuzz-skips: failed, but not for its reason: $(tail -2 "$WORK/fuzz.out" | tr '\n' ' ')")
    echo "MISSED: fuzz-skips failed for another reason"
fi

echo
echo "controls: $ran   caught: $caught   missed: $missed"
if [ ${#MISSED[@]} -gt 0 ]; then
    printf '  %s\n' "${MISSED[@]}"
fi
[ "$missed" -eq 0 ] && [ "$caught" -eq "$ran" ] && [ "$ran" -ge 8 ]
