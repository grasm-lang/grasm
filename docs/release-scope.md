# Release scope, and what blocks one

`docs/stability.md` says what each clause level *promises*. This document says
what a release *requires* — which defects hold a version back and which are
filed and shipped alongside. Without it, every finding of any size blocks
every release, which is how a project spends two weeks fixing help text with
no version to show for it.

## Why this exists

Between 2026-08-04 and 2026-08-16 the language was put through ten rounds of
cold-model evaluation: a fresh model, given only `dist/grasm-language-pack.md`
and the binary, writing a few hundred probe programs and reporting every place
the implementation and the specification disagree. Every round found something,
so a release criterion of "a round that finds nothing" was never met and, for a
surface this size, never would be. Rust, Go and Zig each carry thousands of
open issues; a language is not shipped when its defect count reaches zero, it
is shipped when the defects that remain are known, bounded and outside the
promise being made.

The severity of what those rounds found did fall, and that is the number worth
watching:

| rounds | what was found |
|---|---|
| early | two SIGSEGVs, engine disagreements, wrong answers, a leaked scratch directory per build |
| middle | capability escapes, a normative engine that was the wrong one |
| 40–41 | effect-row accounting, one FFI call spelling, one help text — **no crash, no wrong answer, no engine disagreement in 422 probes and 98 three-engine runs** |

## Severity

**S1 — the implementation is wrong in a way a program can see.** A crash, a
wrong answer, a disagreement between `grasm run`, the bundle and the native
binary that [E-PANIC-TRACE] does not license, or a build that leaks state.

**S2 — a `stable` clause is unsound.** A program the specification forbids is
accepted, and the clause it violates is listed `stable`.

**S3 — an `experimental` or `provisional` clause is unsound, or any clause is
over-strict.** A capability escape through effect-row inference is S3 today,
because [T-CAPABILITY] is `experimental` and says so. A well-typed program
refused is S3 wherever the clause sits: it costs a user a workaround, not a
guarantee.

**S4 — diagnostics.** A message that misdescribes the cause, or help that does
not produce a program that compiles.

## What blocks a release

A version ships when **S1 and S2 are zero**, every gate is green, and every
known S3 and S4 is written down in `docs/known-issues.md` with its clause and
a reproducer.

S3 and S4 do not block. They are the ordinary content of a changelog.

This is a real bar, not a lowered one: S1 covers every property the three
engines are built to guarantee, and S2 covers every promise the stable surface
makes. What it stops doing is holding a release hostage to the parts of the
language that are *documented as unfinished*.

## The 1.0 surface

**In.** Types and inference, functions, pattern matching, collections and the
standard library, modules, fallible values `T ! E`, ownership and `inout`, the
interpreter, the bundle format and the native backend — everything whose
clauses are listed `stable` or `provisional`.

**Out, and documented as out.** Effects and their rows, the capability
ceiling, `par_map`/`par_filter` purity, regions, Tier-2 `ref`, and FFI. Every
clause in those areas is `experimental`: usable, specified, tested, and
explicitly not covered by the compatibility promise.

That split is not a retreat. It is where the evaluation rounds actually found
things: of the defects found in the last four rounds, all but one were in the
`experimental` set, and the effect-row inference alone accounts for most of
them — because it was a syntactic approximation of a type-and-effect system
rather than the thing itself.

That approximation was removed on 2026-08-16. Rows now travel through the type
checker's own unification, and the effect pass reads the checker's answer
instead of reconstructing it: 511 lines of guessing deleted, every gate green,
and the class of defect that produced most of the last four rounds' findings no
longer has a mechanism. The area stays `experimental` until it has sat through
evaluation rounds in its new form — a clause is settled by evidence, not by the
day its rewrite landed.

## Where the evaluation rounds fit

They continue, but scoped: a round's finding is triaged by the table above, and
only S1 and S2 stop a release. A round that finds three S3s in effect-row
inference is a *good* round — it is the rewrite's test suite being written in
advance — and it is not a reason to withhold a version of the stable surface.
