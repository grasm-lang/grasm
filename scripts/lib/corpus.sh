# What the corpus is. One definition, because five gates were carrying their
# own copy of the same four directory names.
#
# A copied list drifts the way a copied count drifts: adding a directory means
# editing every gate that hand-wrote the list, and the one that gets missed is
# a directory nothing checks -- which looks exactly like a directory that
# passes. `scripts/gatecheck.sh` fails any gate that names these directories
# itself instead of sourcing this.
#
# Overridable, because a debug build is held to the same programs without the
# benchmarks: those are sized for a release interpreter and take minutes each
# under a debug one.
CORPUS_DIRS=${CORPUS_DIRS:-"apps examples packages projects spec/conformance benchmarks/grasm"}

# Every standalone program in the corpus, in a stable order.
#
# A directory that has gone missing is reported rather than skipped: `find`
# prints to stderr and carries on, so a renamed directory would quietly shrink
# the corpus while every gate still said PASS.
corpus_programs() {
    local d
    for d in $CORPUS_DIRS; do
        if [ ! -d "$d" ]; then
            echo "corpus directory missing: $d (scripts/lib/corpus.sh)" >&2
            return 1
        fi
    done
    # shellcheck disable=SC2086
    find $CORPUS_DIRS -name '*.gr' | sort
}
