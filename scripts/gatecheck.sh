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
        if printf '%s\n' "$defined" | grep -cx "$name" >/dev/null; then
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

# The second thing this checks: a gate that writes out the corpus directories
# itself instead of sourcing `scripts/lib/corpus.sh`. Five gates were each
# carrying their own copy of the same four names, so adding a directory meant
# editing five files -- and the one that got missed would be a directory
# nothing ran, which is indistinguishable from a directory that passes.
copied=0
for f in scripts/*.sh ProjectGold/run.sh; do
    [ "$f" = "scripts/gatecheck.sh" ] && continue
    [ "$f" = "scripts/lib/corpus.sh" ] && continue
    grep -q 'corpus\.sh' "$f" && continue
    if grep -qE "find [^|]*\bspec/conformance\b[^|]*-name" "$f"; then
        echo "  $f: names the corpus directories itself."
        echo "      Source scripts/lib/corpus.sh and call corpus_programs."
        copied=$((copied + 1))
    fi
done
bad=$((bad + copied))

# `set -o pipefail` together with `… | grep -q …` is a gate that reports the
# opposite of what it found.
#
# `grep -q` exits at the first match. Whatever is upstream is then killed by
# SIGPIPE, `pipefail` takes the pipeline's status from that, and the `if` sees
# failure *because* the pattern matched. It only shows up when the upstream is
# still writing when the match lands, so it hides on short inputs and appears
# on long ones -- in `wasmtest.sh` it made two runs of the same gate disagree,
# and it ran programs on a target with no filesystem while reporting them as
# disagreements rather than skips.
#
# `grep -c PATTERN >/dev/null` has the same exit status and reads to the end,
# so there is no early exit to be killed for.
piped=0
for f in scripts/*.sh ProjectGold/run.sh; do
    [ "$f" = "scripts/gatecheck.sh" ] && continue
    grep -cE '^[[:space:]]*set[[:space:]].*pipefail' "$f" >/dev/null || continue
    if grep -cE '\|[[:space:]]*grep[[:space:]]+-[a-zA-Z]*q' "$f" >/dev/null; then
        echo "  $f: pipes into \`grep -q\` under \`pipefail\`."
        echo "      SIGPIPE makes the pipeline fail when the pattern matches."
        echo "      Use \`grep -c PATTERN >/dev/null\` instead."
        piped=$((piped + 1))
    fi
done
# A gate `allgates.sh` runs by its path has to be executable, here and in
# git. Eleven runtime fuzz gates were committed 0644 on 2026-10-28: run as
# `bash scripts/x.sh` while they were written, each passed, and the first
# suite run to reach them got "Permission denied", exit 126, from every one.
notexec=0
for path in $(grep -oE '^run_gate +[a-z0-9_]+ +(scripts|ProjectGold)/[A-Za-z0-9_./-]+' scripts/allgates.sh | awk '{print $3}' | sort -u); do
    mode=$(git ls-files -s -- "$path" | awk '{print $1}')
    if [ ! -x "$path" ] || [ "$mode" != 100755 ]; then
        echo "  $path: allgates.sh runs it by its path, and it is not executable (git mode ${mode:-untracked})."
        notexec=$((notexec + 1))
    fi
done

# Kept apart from the total. The summary used to print the running total
# under the label `undefined`, so one `grep -q` finding read as "undefined: 1"
# and sent a reader looking for a helper that did not exist -- the count was
# right and the name was wrong, which is worse than a wrong count because it
# is believed.
undefined=$bad
bad=$((bad + piped + notexec))

echo "=========================================="
echo "gate helpers checked: $checked   undefined: $undefined   corpus copied: $copied   grep -q under pipefail: $piped   gates not executable: $notexec"
echo "=========================================="
[ "$bad" = 0 ]
