# 29 — Self-Hosting Compiler Segment
**Tests:** compiler maturity, bootstrapping. **Result:** works — **25 of 25 files
lex identically to the reference implementation.**

## What this is

A Grasm lexer, written in Grasm, tokenizing the language it is written in. Not a
toy subset: it handles the things that make the real lexer non-trivial, each of
which was a defect in it at some point and carries a comment saying so.

* **nested block comments** — `/* a /* b */ still */`. Scanning for the first
  `*/` swallows the rest of the file, and these nest by design.
* **`///` versus `//` versus `////`** — documentation, comment, comment.
* **string interpolation** — `{` opens an expression whose contents are not
  string characters; `\{` is a literal brace.
* **`t.1.0`** — a `.` after a number is a fraction *unless* the previous token
  was itself a `.`, in which case this is a tuple component. Max-munch reads
  `t.1.0` as `t . 1.0`, and a nested tuple component then has no unparenthesised
  spelling at all.
* **newline runs collapse** to one token.

## The claim, and how it is checked

`counts.txt` holds the reference lexer's token count (`grasm tokens`) for every
program in this directory, and the program compares its own count against it.
**All match exactly.** `run.sh` regenerates that table before every gate
run, because a stale table turns "the self-hosted lexer disagrees" into
"somebody edited a program", and those are not the same finding.

A self-hosted component that has not been diffed against the thing it replaces
is a rewrite, not a bootstrap.

## What the diff found

Two real defects in this lexer, neither of which reading would have caught:

1. **Doc comments were emitted as tokens.** The reference keeps them out of the
   stream entirely — and a `TDoc` in the stream also stops consecutive newlines
   collapsing across it, so a two-line doc comment produced five tokens where the
   reference produces two. Files with no doc comments already matched exactly;
   this is what made the rest match.
2. **`??` was missing from the two-character operator table**, so it lexed as two
   `?`. Three files were two or three tokens long, and the counts matched the
   number of `??` in each exactly.

It lexes its own 14,415-character source with **zero** lexical errors.

## Findings
None in the language. Both findings were in this program, and the reference
comparison is what surfaced them.
