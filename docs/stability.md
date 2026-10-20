# Stability policy

Grasm is at 1.0. This document says exactly what that commits to, so that
"1.0" is a bounded promise rather than a mood.

1.0 commits to the clauses listed `stable`, and to nothing else. The other two
levels mean today what they meant before it: `provisional` may change in a
minor release with a migration note, `experimental` in any release. Raising the
version number promoted nothing — a clause is promoted when it passes the
five-part test below, one at a time, on its own evidence.

## What has moved, and why

Moved here from `spec/stability.md`, which lists the levels and is held to a
page budget; these are the record of two decisions rather than a rule.

**Five clauses were demoted from `stable` to `experimental` on 2026-08-16**:
`T-FN-TYPE-ROW`, `T-FN-TYPE-ROW-PARAM`, `T-FN-TYPE-ROW-CARRIED`,
`T-FN-VALUE-ROW` and `T-UNSAFE-UFCS`. Their meaning had not changed; their
implementation moved in nearly every evaluation round, which is what
criterion 5 forbids. Saying so was cheaper than defending the listing.

**53 clauses promoted, 2026-09-10 and 2026-09-13**, each on its evidence against
the five-part test below. The audit is reproducible and 26
clauses did not pass it; the commit that promoted these says why each failed.

The unit of stability is the **normative clause**: every rule in `spec/` carries
a tag like `[E-MAP-ORDER]`, and every tag has a stability level recorded in
`spec/stability.md`. A clause with no recorded level fails the test suite, so
this document cannot drift out of sync with the specification the way a
hand-maintained list would.

`docs/release-scope.md` says which of these levels a release is allowed to have
open defects in, and `docs/known-issues.md` lists the ones that are open today.

## Levels

**`stable`** — will not change incompatibly before 2.0. A program relying on a
stable clause keeps working. Fixing an implementation that *disagrees* with a
stable clause is not a breaking change; the clause is the contract, not the bug.

**`provisional`** — the behaviour is specified and tested, but the design is
still being validated against real programs. It may change in a minor release
with a migration note in the changelog. Most of the language was here at 1.0;
after the promotions of 2026-09-10 it is 30 clauses of 239, and what is left
is there for a reason the promotion audit records.

**`experimental`** — may change or be removed in any release, without a
deprecation period. New surface starts here.

## What gets promoted, and when

A clause moves from `provisional` to `stable` when all five hold. The test is
written down so the decision is auditable rather than a matter of taste, and
so anyone can check it against the repository:

1. It has a conformance program, and that program passes under both engines.
2. **Its meaning has never changed.** Not "has not changed lately" — the
   program's history shows edits that added coverage, never ones that reversed
   a behaviour.
3. It is not the newest work. A clause written this month has been used by
   nobody, and "we have not found a problem" is not evidence when there has
   been no opportunity to find one.
4. It names nothing the implementation admits is unfinished.
5. It is not in an area with open design work. A clause is not settled because
   *it* has not moved; it is unsettled if the thing around it is moving.

Rules 3 and 5 are what keep this honest. Everything else here would promote
the whole specification on the grounds that a single author has not yet
contradicted himself.

## What "breaking" means

A change is breaking if a program that compiled and ran correctly before now
fails to compile, or produces different output. By that definition these are
**not** breaking:

- Making the compiler accept a program it used to reject.
- Making a *diagnostic* clearer, or changing its wording.
- Changing performance, memory use, or the machine code emitted.
- Changing anything under `crates/` that is not reachable from the CLI or the
  specification. The Rust crates are implementation detail, not a public API.
- Fixing the interpreter or the native backend to match a stable clause.

These are:

- Removing or renaming a builtin, method, keyword, or CLI flag.
- Changing the meaning of a stable clause.
- Changing the output of a program that only uses stable behaviour.

## Versioning

`MAJOR.MINOR.PATCH`, and the version applies to the toolchain as a whole — the
language, the CLI, the specification, and the file formats (`grasm.toml`,
`grasm.lock`) move together. Splitting them would let the language and the tool
that implements it drift apart, which is the problem the conformance suite
exists to prevent.

- **PATCH** — bug fixes; no clause changes level or meaning.
- **MINOR** — new clauses, new builtins, new CLI surface. Provisional and
  experimental clauses may change here, with a changelog entry.
- **MAJOR** — stable clauses may change. Before 1.0, the MINOR number carries
  that weight instead: 0.x → 0.(x+1) may break anything.

## Reaching 1.0

1.0 is not a date, it is a checklist:

1. Every clause in `spec/` is `stable` or deliberately, documentedly not.
   Today: **91 stable, 20 provisional, 16 experimental**, of 127. The twenty
   are the ones that fail a promotion rule and say which — library
   inventories that will grow, the effect system where this month's work
   happened, generic bounds as the known next step, and the clauses written
   too recently for "no problem found" to mean anything.
2. The native backend compiles every program the interpreter runs. It does
   today — `scripts/difftest.sh` reports 130 identical and 0 unsupported — and
   staying there is the requirement: any construct the backend cannot compile
   must name itself so difftest reports it rather than skipping quietly.
3. Multi-core execution, which has since been answered rather than reached.
   `parallel` and `spawn` stay deterministic and single-threaded on purpose
   (`[E-NOT-PARALLEL]`): they exist to structure concurrency, and replaying
   the same interleaving every run is worth more there than throughput.
   Real parallelism is `par_map` and `par_filter`, where the effect system
   proves the body is pure before any thread is created. So this line is no
   longer a gap to close but a decision to keep documented.
4. The package manager resolves versions, not just paths. **Done.** A registry
   is a directory of `<name>/<version>/` packages; selection is minimal
   version selection, so the result does not depend on walk order and adding
   a dependency cannot silently upgrade one you already had. What remains is
   distribution — fetching and publishing — which is infrastructure rather
   than language design, and is deliberately outside the toolchain.
5. A written migration story for each provisional clause that changed on the
   way there.

## Deprecation

Removing something stable takes two minor releases: first the compiler warns
and the changelog says what to use instead; then it goes. Nothing stable is
removed without a warning release in between.

## Security

See `SECURITY.md`. Security fixes ship in a PATCH release and may change
behaviour that a stable clause describes if the clause itself is the problem —
that case is called out explicitly in the changelog rather than folded in.
