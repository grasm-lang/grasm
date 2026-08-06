# Clause stability

The level of every normative clause in `spec/`. See `docs/stability.md` for
what each level promises, and for the five-part test a clause has to pass
before it is listed as stable.

`crates/grasm_spec` fails if a clause here is missing, unknown, or listed twice,
so this file cannot drift away from the specification it describes.

Today: 94 stable, 33 provisional, 16 experimental, of 143.

## Stable

Will not change incompatibly before 2.0.

- `E-ARGS` — stable
- `E-ARITH-FLOAT` — stable
- `E-ARITH-INT` — stable
- `E-CLOSURE-CAPTURE` — stable
- `E-COALESCE-LAZY` — stable
- `E-COW` — stable
- `E-DIV-ZERO` — stable
- `E-ENTRY` — stable
- `E-EQ-STRUCTURAL` — stable
- `E-ERR-PROPAGATE` — stable
- `E-HANDLER-INNERMOST` — stable
- `E-HANDLER-MISSING` — stable
- `E-HANDLER-SCOPE` — stable
- `E-IF-VALUE` — stable
- `E-INDEX-BOUNDS` — stable
- `E-INOUT-WRITEBACK` — stable
- `E-LOOP-LIST` — stable
- `E-LOOP-RANGE` — stable
- `E-MAIN-ERR` — stable
- `E-MAP-ORDER` — stable
- `E-MATCH-ORDER` — stable
- `E-MINMAX-OPTION` — stable
- `E-NUM-PROMOTE` — stable
- `E-ORDER-LTR` — stable
- `E-OWN-RUNTIME` — stable
- `E-PAD-CYCLE` — stable
- `E-PANIC-EXIT` — stable
- `E-POW-NEG` — stable
- `E-SEARCH-SHORTCIRCUIT` — stable
- `E-SHORTCIRCUIT` — stable
- `E-STR-CHARS` — stable
- `E-STR-COMPARE` — stable
- `E-STR-CONCAT` — stable
- `E-VALUE-SEMANTICS` — stable
- `G-ARM-CONTROL` — stable
- `G-DECL-ORDER` — stable
- `G-PAT-BIND` — stable
- `G-PIPE` — stable
- `G-STMT-LAST` — stable
- `G-STMT-TERM` — stable
- `G-STRUCT-AMBIG` — stable
- `G-TYPE-UNIT` — stable
- `L-BOOL` — stable
- `L-COMMENT-BLOCK` — stable
- `L-COMMENT-LINE` — stable
- `L-CONTEXTUAL` — stable
- `L-ENCODING` — stable
- `L-ESCAPE` — stable
- `L-FLOAT` — stable
- `L-IDENT` — stable
- `L-INT` — stable
- `L-INTERP` — stable
- `L-INTERP-ONE` — stable
- `L-KEYWORD` — stable
- `L-MAXMUNCH` — stable
- `L-NEWLINE` — stable
- `L-STRING` — stable
- `L-WHITESPACE` — stable
- `T-ARM-TYPES` — stable
- `T-CAPTURE-WRITE` — stable
- `T-COPY` — stable
- `T-EMPTY-LIT` — stable
- `T-ERR-SUBSET` — stable
- `T-FAIL` — stable
- `T-FALLIBLE` — stable
- `T-FALLIBLE-UNHANDLED` — stable
- `T-INFER-LOCAL` — stable
- `T-INOUT-EXCLUSIVE` — stable
- `T-INOUT-MUTABLE` — stable
- `T-INOUT-PLACE` — stable
- `T-INT-TO-FLOAT` — stable
- `T-LET-IMMUTABLE` — stable
- `T-LIST` — stable
- `T-MAP` — stable
- `T-MAP-KEY` — stable
- `T-MATCH-EXHAUSTIVE` — stable
- `T-MATCH-GUARD` — stable
- `T-MODULE-NS` — stable
- `T-MODULE-PUB` — stable
- `T-MODULE-RESOLVE` — stable
- `T-MOVE-LOOP` — stable
- `T-NO-NULL` — stable
- `T-OPTION` — stable
- `T-PARAM-INOUT` — stable
- `T-PARAM-LET` — stable
- `T-PARAM-OWN` — stable
- `T-PRIM` — stable
- `T-RECURSIVE` — stable
- `T-STRUCT` — stable
- `T-SUM` — stable
- `T-TRY` — stable
- `T-TUPLE` — stable
- `T-USE-AFTER-MOVE` — stable
- `T-VAR-MUTABLE` — stable

## Provisional

Specified and tested; the design is still being validated against real
programs. May change in a minor release, with a changelog entry.

These are the clauses that fail one of the promotion rules: the library
inventories will grow, the effect system is where this month's work happened,
generic bounds are the known next step, `T-INT-ALIAS` names types the
implementation admits are unfinished, and the rest were written too recently
for "no problem found" to mean anything.

- `E-LIST-CONCAT` — provisional
- `E-MAP-LIT` — provisional
- `E-RECURSION-DEPTH` — provisional
- `G-LET-TUPLE` — provisional
- `G-MAP-BLOCK-AMBIG` — provisional
- `G-NEST-LIMIT` — provisional
- `L-DISPLAY` — provisional
- `L-LIB-CODES` — provisional
- `L-LIB-FREE` — provisional
- `L-LIB-IO` — provisional
- `L-LIB-LIST` — provisional
- `L-LIB-MAP` — provisional
- `L-LIB-NUM` — provisional
- `L-LIB-OPTION` — provisional
- `L-LIB-PAIRS` — provisional
- `L-LIB-SORT-BY` — provisional
- `L-LIB-STR` — provisional
- `L-UFCS` — provisional
- `T-DIVERGE` — provisional
- `T-EFFECT-DECL` — provisional
- `T-EFFECT-DISCHARGE` — provisional
- `T-EFFECT-FALLIBLE` — provisional
- `T-EFFECT-ROW` — provisional
- `T-FALLIBLE-COALESCE` — provisional
- `T-FALLIBLE-VALUE` — provisional
- `T-FN-TYPE` — provisional
- `T-FN-TYPE-ROW` — provisional
- `T-FN-VALUE` — provisional
- `T-GENERIC-FN` — provisional
- `T-GENERIC-INST` — provisional
- `T-INT-ALIAS` — provisional
- `T-MATCH-ALT` — provisional
- `T-MODULE-DEP` — provisional

## Experimental

May change or be removed in any release. New surface starts here.

- `E-BENCH-WARMUP` — experimental
- `E-NOT-PARALLEL` — experimental
- `E-PAR-FILTER-ORDER` — experimental
- `E-PAR-MAP-ORDER` — experimental
- `E-PARALLEL-JOIN` — experimental
- `E-SPAWN-ORDER` — experimental
- `E-SPAWN-SHARED` — experimental
- `E-TASK-PANIC` — experimental
- `E-TEST-PROPERTY` — experimental
- `E-TEST-SEED` — experimental
- `E-TEST-SHRINK` — experimental
- `G-BENCH-CONTEXTUAL` — experimental
- `G-TEST-PARAMS` — experimental
- `T-EFFECT-BUILTIN` — experimental
- `T-PAR-MAP-PURE` — experimental
- `T-TASK-CAPTURE` — experimental
