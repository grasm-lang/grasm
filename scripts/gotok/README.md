# Go token counter

Counts tokens with Go's own lexer (`go/scanner`), for the [G11] concision
measurement in `scripts/northstar.sh`.

A regular expression cannot count these three languages against each other
fairly. Python's grammar has `INDENT`, `DEDENT` and `NEWLINE` tokens; Go's lexer
inserts semicolons. A regex over punctuation gives both of those away free and
charges Grasm for every brace. Measured that way Grasm read 1.22x of Python;
measured with each language's own lexer it reads 1.04x. The difference was the
ruler, not the languages.

Built on demand, in a scratch module, and only when a Go toolchain is present —
without one the Go column is skipped rather than approximated.
