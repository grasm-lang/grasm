# grlint

A linter for Grasm, written in Grasm.

Phase 3 of `docs/apps-phases.md`: the question the other phases could not ask
is whether the language holds up for somebody who is *not* writing a demo. The
nearest available substitute for an outside user is a tool that has to read
code nobody wrote for it — so this runs over this repository's own 358
programs, and anything it cannot handle is a finding rather than an excuse.

It is also self-hosting pressure at a size that can be finished. A compiler
written in its own language is the standard proof; a *linter* written in its
own language exercises the same thing — reading Grasm at scale — today.

```sh
grasm build apps/grlint/src/main.gr --native -o grlint

./grlint                       # the self-test: every rule, both directions
./grlint --summary packages    # counts per rule
./grlint --only empty-catch .  # one rule
./grlint --json packages       # for anything that reads it mechanically
```

## The rules

| rule | |
|---|---|
| `long-line` | over 100 characters |
| `trailing-space` | whitespace at the end of a line |
| `tab-indent` | a tab where `grasm fmt` uses four spaces |
| `todo` | a TODO or FIXME left in |
| `empty-catch` | `Err(e) -> {}` — a failure named and discarded |
| `unwrap-default` | `?? 0` on a parse, where 0 and "could not parse" become one answer |
| `wide-row` | a function declaring more than three effects |
| `no-doc` | a `pub fn` with no `///` above it |

Every rule is one a *line* can answer. The ones that need the syntax tree are
named by `unsupported()` rather than approximated, because a rule guessed at
with a regular expression is wrong on exactly the programs that matter — the
ones whose shape is unusual.

## What it found

**640 findings over 358 programs, in 0.57 s.**

| | |
|---|---|
| `long-line` | 278 |
| `no-doc` | 215 |
| `empty-catch` | 113 |
| `unwrap-default` | 22 |
| `todo` | 4 |
| `wide-row` | 4 |
| `tab-indent` | 3 |
| `trailing-space` | 1 |

Four of the `empty-catch` findings were in `packages/` and `apps/` — code
written in this session — and **all four were real**. A write to a closed
socket and a partial parse are both deliberate, and all four were written
`Err(e) -> {}`: a name bound and thrown away, which reads as an oversight.
They say `Err(_gone)`, `Err(_hung_up)`, `Err(_incomplete)` now, and each
carries the sentence explaining why nothing happens.

That is the rule earning its place: it did not find a bug, it found four
places where correct code did not say it was correct.

## What building it found

**A rule that checked the wrong thing.** `no-doc` is "a `pub fn` with no
`///` above it", and the first version asked `starts_with("//")` — which every
ordinary comment satisfies. So the rule fired on nothing that had a comment of
any kind above it, which is most exported functions, and the self-test recorded
that as correct. Caught by reading the self-test's output rather than by the
self-test itself, which is worth saying: a test that asserts what the code does
agrees with the code.

**An invocation that was wrong, and a specification that was right.**
`./grlint -- --summary` silently ignored every flag. [E-ARGS] says a compiled
binary has no `--` separator — the `--` in `grasm run prog.gr -- a b` belongs
to the toolchain — so the literal `--` arrived as an argument and `cli` read
everything after it as positional, which is exactly what `--` means to a
command line. Correct behaviour by both, and a thing a first-time user gets
wrong once.

## What it does not do

It is not a parser. A rule that needs scope — an unused binding, shadowing —
needs the syntax tree, and the compiler has one. Extending this to use the
compiler's tree would be the natural next step and is a different program.
