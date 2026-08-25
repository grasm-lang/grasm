# The one definition of a panic's stack trace block.
#
# [E-PANIC-TRACE] lets the engines differ about the trace and nothing else, so
# every cross-engine gate strips it before comparing stderr. Three of them had
# their own copy of the rule, and `crosstest.sh`'s was missing the third
# pattern -- so the interpreter's "... N more frame(s)" survived the strip and
# `E-PANIC-TRACE.gr` was reported as a cross-architecture mismatch when the two
# engines were doing exactly what the clause says they do.
#
# Reads the text as `$1` and writes the stripped form to stdout.
strip_trace() {
    grep -v \
        -e '^stack trace (innermost first):$' \
        -e '^  [A-Za-z_][A-Za-z0-9_:]* at ' \
        -e '^  \.\.\. [0-9]* more frame(s)$' <<<"$1"
}
