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

# Whether a program needs a system library on the link line.
#
# A program whose package graph declares `links` calls an `extern fn` that the
# system linker resolves, so two separate things are true of it:
#
#   * it cannot be built without that library's search path, which is a
#     property of the machine rather than of the code;
#   * it cannot be compared against the interpreter at all, because
#     `grasm run` reaches only symbols already loaded in its own process.
#
# Asked of the compiler -- `grasm links` -- so no gate carries a list of
# filenames that rots, and none of them matches on a build failure's text,
# which would make the gate agree with whatever the message happens to say.
#
# One definition because three gates needed it: `difftest.sh` found the hole,
# and `leakcheck.sh` and `crosstest.sh` had the same one. A copied rule drifts
# the way a copied count drifts.
corpus_external() {
    [ -n "$("${GRASM:-./target/release/grasm}" links "$1" 2>/dev/null)" ]
}

# Fails unless every program named in "$@" is one `scripts/sdltest.sh` builds.
#
# An excluded program is only excluded honestly if something else checks it.
# Counting the excluded and printing the count is what F76 was about: the
# number was there and nobody compared it to anything. So a new program that
# calls into C and that nobody gated fails the gate that skipped it.
corpus_external_covered() {
    local covered prog
    # One padded string matched with `case`, not a pipe into `grep -q`: under
    # `pipefail` that fails the pipeline when the pattern *matches*, so a
    # covered program would read as uncovered.
    covered=" $(grep -oE 'packages/[a-z0-9-]+/src/main\.gr' scripts/sdltest.sh |
        sort -u | tr '\n' ' ')"
    for prog in "$@"; do
        case "$covered" in
            *" $prog "*) continue ;;
        esac
        echo "    $prog calls into a C library and no gate checks it."
        echo "    Add it to scripts/sdltest.sh, or this is unchecked code."
        return 1
    done
    return 0
}

# A per-program wall clock for the gates that run every corpus program.
#
# Closing stdin stops the one hang that happened; this stops the class. A
# program that loops forever -- or waits on something that is not stdin --
# otherwise stalls the gate, and these gates buffer their output until the
# end, so the symptom is an hour of silence rather than a failure. That is
# what `packages/rmcp-demo` did, and the next one will not be a server.
#
# `timeout` is not on a stock macOS, so this is empty when it is absent
# rather than a new requirement. Unquoted on purpose: empty must expand to
# no argument at all.
#
# The limit is deliberately far above anything legitimate, because it exists
# to turn "forever" into "a failure" and not to police speed. A performance
# budget belongs in a bench gate, where a number means something; here a
# tight limit only produces false failures on work that is slow on purpose.
#
# 120s was the first guess and it failed three real programs, each of them
# under the interpreter while the native build finished. Measured rather
# than guessed the second time, on a machine at a load average near 20:
#
#     packages/crypto_hash-demo     134s
#     examples/parallel_map.gr      144s
#     packages/password-demo        163s
#
# A password KDF is expensive by design -- being slow is the feature -- and
# it is slower again interpreted. 1800s is eleven times the slowest of them
# and still bounds a hang, which is the whole difference from the hour of
# silence this replaced. Raise it before trimming it: a false failure here
# costs a two-hour rerun, and the only thing a tight limit buys is finding a
# hang sooner than nobody needs it found.
# The number lives here and the gates' messages read it back, so raising it
# does not leave four "after 120s" lines saying something that is no longer
# true.
RUN_LIMIT_SECONDS=${GRASM_RUN_LIMIT:-1800}
if command -v timeout >/dev/null 2>&1; then
    RUN_LIMIT="timeout $RUN_LIMIT_SECONDS"
else
    RUN_LIMIT=""
fi
