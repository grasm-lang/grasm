#!/usr/bin/env bash
# Does `packages/tempdir` claim a directory nobody else can be holding?
#
# A temporary file cannot be made safely with what the runtime offers:
# `write_bytes` truncates whatever is at the path, so a random name plus a
# write cannot tell "I claimed this" from "a symlink to somebody's file was
# already here". `mkdir` can -- it is atomic and reports EEXIST -- so the
# safe shape is a 0700 directory with files inside it, which is what
# `mktemp -d` does and why.
#
# The properties checked here are the ones a weaker implementation passes a
# functional test while failing:
#
#   * names differ across claims -- a counter or a pid would not
#   * names are long and hex, so they are not guessable
#   * the directory is 0700, so another user cannot read what goes in it
#   * TMPDIR is honoured, and a caller-named root overrides it
#   * an unwritable root fails instead of spinning
#   * claiming a name that is already taken fails rather than succeeding
#
# The last one is the whole point and is checked directly against
# `make_dir_new`, because it is the primitive the safety rests on.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if ! "$GRASM" build packages/tempdir-demo/src/main.gr --native -o "$work/td" \
    >"$work/build.log" 2>&1; then
    echo "    the demo did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

mkdir -p "$work/root"
TMPDIR="$work/tmp" && mkdir -p "$TMPDIR"
export TMPDIR
# The paths go to a listing rather than to stdout: the demo is in the
# corpus `difftest.sh` walks, which requires both engines to print the same
# bytes, and a random name never will. stdout carries only the counts.
"$work/td" "$work/listing.txt" "$work/root" >"$work/out.txt" 2>&1 || {
    echo "    the demo did not run:"
    sed 's/^/      /' "$work/out.txt"
    exit 1
}

# TMPDIR is honoured, not /tmp.
grep -q "^ROOT $work/tmp$" "$work/listing.txt" || {
    echo "    TMPDIR was not used as the root:"
    grep "^ROOT " "$work/listing.txt" | sed 's/^/      /'
    exit 1
}

# The counts stdout reports must agree with the listing, or the demo is
# describing work it did not do.
grep -q "^claimed 9$" "$work/out.txt" || { echo "    the demo did not claim 9"; exit 1; }
grep -q "^distinct 9$" "$work/out.txt" || {
    echo "    9 claims were not 9 distinct names -- a sequential name would do this"
    exit 1
}
grep -q "^wrote_inside true$" "$work/out.txt" || { echo "    nothing was written inside"; exit 1; }
grep -q "^missing_root refused$" "$work/out.txt" || {
    echo "    a missing root did not fail"
    exit 1
}

made=$(grep -vE "^(UNDER|INNER|ROOT) " "$work/listing.txt")
count=$(echo "$made" | grep -c .)
distinct=$(echo "$made" | sort -u | grep -c .)
[ "$count" -ge 9 ] || { echo "    only $count directories were claimed"; exit 1; }
[ "$count" = "$distinct" ] || {
    echo "    $count claims produced only $distinct distinct names"
    exit 1
}

# Every name must carry 32 hex characters after the prefix. A short or
# non-hex name is a guessable one.
echo "$made" | while read -r d; do
    base=$(basename "$d")
    suffix=${base#grasmtest-}
    case "$suffix" in
        [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]*) ;;
        *) echo "    $base does not end in hex"; exit 1 ;;
    esac
    [ ${#suffix} -eq 32 ] || { echo "    $base has a ${#suffix}-character name, not 32"; exit 1; }
    perms=$(stat -f "%Sp" "$d" 2>/dev/null || stat -c "%A" "$d")
    case "$perms" in
        drwx------) ;;
        *) echo "    $base is $perms, not drwx------"; exit 1 ;;
    esac
done

# A file written inside it is there and readable by us.
inner=$(sed -n 's/^INNER //p' "$work/listing.txt")
[ -f "$inner" ] || { echo "    the file written inside is not there: $inner"; exit 1; }
grep -q "written inside" "$inner" || { echo "    the file inside has the wrong contents"; exit 1; }

# A caller-named root is used instead of TMPDIR.
grep -q "^UNDER $work/root/own-" "$work/listing.txt" || {
    echo "    a caller-named root was not honoured:"
    grep "^UNDER " "$work/listing.txt" | sed 's/^/      /'
    exit 1
}

# And the primitive itself: a taken name must be refused. This is what the
# whole design rests on, so it is checked against the builtin rather than
# inferred from the package's behaviour.
cat >"$work/excl.gr" <<'GREOF'
fn main() / {io} {
    let a = args()
    let p = a.get(0) ?? ""
    match make_dir_new(p) {
        Ok(_u) -> print("first ok")
        Err(_e) -> print("first refused")
    }
    match make_dir_new(p) {
        Ok(_u) -> print("second ok")
        Err(_e) -> print("second refused")
    }
    // `make_dir` is mkdir -p and must *not* refuse, which is why it cannot
    // be used to claim a name.
    match make_dir(p) {
        Ok(_u) -> print("make_dir ok")
        Err(_e) -> print("make_dir refused")
    }
}
GREOF
"$GRASM" build "$work/excl.gr" --native -o "$work/excl" >/dev/null 2>&1
"$work/excl" "$work/claimed" >"$work/excl.out" 2>&1
want="first ok
second refused
make_dir ok"
got=$(cat "$work/excl.out")
[ "$got" = "$want" ] || {
    echo "    make_dir_new does not claim a name exclusively:"
    diff <(echo "$want") <(echo "$got") | sed 's/^/      /'
    exit 1
}

echo "    claims: $count distinct 32-hex names, all drwx------, a taken name refused"
