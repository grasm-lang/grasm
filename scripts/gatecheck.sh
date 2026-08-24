#!/usr/bin/env bash
# The gates' own gate: a case that names a helper which does not exist.
#
# `scripts/nativegaps.sh` defines `expect_compiled`, and eleven cases were
# written as `expect_compiles`. Bash reported each as a command not found on
# stderr, returned 127, and — with no `set -e` — carried on. Nothing set the
# failure flag, and the script printed "the gap list matches the compiler"
# having run 28 of its 39 cases (F40).
#
# The shape is narrow and easy to find: a line beginning at column 0 with an
# identifier followed by a quoted argument is a gate case. If that identifier is
# neither a function the script defines nor a real command, the case does
# nothing and says nothing.
#
# `command_not_found_handle` would catch this at run time in one line, and bash
# 3.2 — which is what macOS ships and what these run under — does not have it.
# So the check is static, and it is the one that would have caught F40 the
# moment it was written rather than a day later.
set -uo pipefail
cd "$(dirname "$0")/.."

bad=0
checked=0
for f in scripts/*.sh ProjectGold/run.sh; do
    [ "$f" = "scripts/gatecheck.sh" ] && continue
    # Definitions from the script, and from anything it sources — a helper
    # shared by every gate lives in scripts/lib/, and is no less defined for
    # being defined once.
    defined=$(grep -oE '^[a-z_][a-z_0-9]*\(\)' "$f" | tr -d '()')
    for lib in $(grep -E '^(\.|source) ' "$f" | grep -oE '[A-Za-z0-9_./-]+\.sh'); do
        base=${lib##*/}
        for cand in "scripts/lib/$base" "scripts/$base" "$lib"; do
            [ -f "$cand" ] || continue
            defined="$defined
$(grep -oE '^[a-z_][a-z_0-9]*\(\)' "$cand" | tr -d '()')"
            break
        done
    done
    while IFS= read -r name; do
        [ -z "$name" ] && continue
        checked=$((checked + 1))
        if printf '%s\n' "$defined" | grep -qx "$name"; then
            continue
        fi
        if command -v "$name" >/dev/null 2>&1; then
            continue
        fi
        echo "  $f: \`$name\` is called with arguments but is not defined here"
        echo "      and is not a command — every case using it does nothing."
        bad=$((bad + 1))
    done <<EOF
$(grep -oE "^[a-z_][a-z_0-9]* +[\"']" "$f" | sed 's/ *["'"'"']$//' | sort -u)
EOF
done

echo "=========================================="
echo "gate helpers checked: $checked   undefined: $bad"
echo "=========================================="
[ "$bad" = 0 ]
