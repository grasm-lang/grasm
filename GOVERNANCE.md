# Governance

Grasm is early and small. This describes how it is run today, honestly, rather
than describing a foundation that does not exist.

## Today

The project is maintained by its authors. Decisions are made by whoever is
doing the work, in the open, in the repository. There is no committee, no RFC
process, and no voting, because there is not yet enough traffic for any of
those to be anything but ceremony.

What there *is*:

- **`spec/` is the source of truth.** A behaviour is decided when it has a
  tagged clause and a conformance program. Discussion happens in issues and
  pull requests; the outcome lands in the specification. Anything not in the
  specification is not a promise.
- **`docs/stability.md` bounds what can change.** Stable clauses do not change
  incompatibly before 2.0. That constraint applies to maintainers first.
- **The gates apply to everyone.** Nobody merges past a failing conformance
  suite or a red differential test, including whoever wrote them.

## Proposing a language change

Open an issue with the clause you would add or change, written the way the
existing clauses are written, and a program showing the difference. What makes
a proposal convincing here is not novelty: it is that the rule can be stated in
one paragraph, tested by one program, and implemented in both engines.

Proposals that make the language smaller are especially welcome. The kernel is
meant to stay small enough to hold in your head; every construct added has to
pay for the room it takes up.

## When this changes

If Grasm gets enough contributors that "whoever is doing the work" stops
scaling, this document gets replaced with something with names in it. Until
then, pretending otherwise would be worse than saying so.

## Security

Report vulnerabilities privately — see `SECURITY.md`. Security fixes are the one
case where a change may land without the usual discussion.
