# Clause stability

The level of every normative clause in `spec/`. See `docs/stability.md` for
what each level promises, and for the five-part test a clause has to pass
before it is listed as stable.

`crates/grasm_spec` fails if a clause here is missing, unknown, or listed twice,
so this file cannot drift away from the specification it describes.

**Five clauses were demoted from `stable` to `experimental` on 2026-08-16**:
`T-FN-TYPE-ROW`, `T-FN-TYPE-ROW-PARAM`, `T-FN-TYPE-ROW-CARRIED`,
`T-FN-VALUE-ROW` and `T-UNSAFE-UFCS`. Their meaning had not changed; their
implementation moved in nearly every evaluation round, which is what
criterion 5 forbids. Saying so was cheaper than defending the listing.

**53 clauses promoted, 2026-09-10 and 2026-09-13**, each on its evidence against
the five-part test in `docs/stability.md`. The audit is reproducible and 26
clauses did not pass it; the commit that promoted these says why each failed.

Today: 167 stable, 26 provisional, 62 experimental, of 255.

## Stable

Will not change incompatibly before 2.0.

- `E-ARGS` — stable
- `E-ARITH-FLOAT` — stable
- `E-ARITH-INT` — stable
- `E-BORROWED-READ` — stable
- `E-COALESCE-DIVERGE` — stable
- `E-COALESCE-LAZY` — stable
- `E-CONST-REF` — stable
- `E-COW` — stable
- `E-DIV-ZERO` — stable
- `E-EFFECT-NO-INOUT` — stable
- `E-ENTRY` — stable
- `E-EQ-STRUCTURAL` — stable
- `E-ERR-PROPAGATE` — stable
- `E-HANDLER-INNERMOST` — stable
- `E-HANDLER-OPS` — stable
- `E-HANDLER-SCOPE` — stable
- `E-IF-DIVERGE` — stable
- `E-IF-VALUE` — stable
- `E-INDEX-BOUNDS` — stable
- `E-INOUT-FAIL` — stable
- `E-INOUT-WRITEBACK` — stable
- `E-LIST-CONCAT` — stable
- `E-LOOP-LIST` — stable
- `E-LOOP-RANGE` — stable
- `E-LOOP-RANGE-MAX` — stable
- `E-MAIN-ERR` — stable
- `E-MAP-LIT` — stable
- `E-MAP-ORDER` — stable
- `E-MATCH-BUILTIN-ARMS` — stable
- `E-MATCH-ORDER` — stable
- `E-MINMAX-OPTION` — stable
- `E-NUM-PROMOTE` — stable
- `E-ORDER-LTR` — stable
- `E-OWN-RUNTIME` — stable
- `E-PAD-CYCLE` — stable
- `E-PANIC-EXIT` — stable
- `E-PANIC-TRACE` — stable
- `E-POW` — stable
- `E-POW-NEG` — stable
- `E-PRINT-ATOMIC` — stable
- `E-RECURSION-PARALLEL` — stable
- `E-SEARCH-SHORTCIRCUIT` — stable
- `E-SHORTCIRCUIT` — stable
- `E-STR-CHARS` — stable
- `E-STR-COMPARE` — stable
- `E-STR-CONCAT` — stable
- `E-VALUE-SEMANTICS` — stable
- `G-ARM-CONTROL` — stable
- `G-DECL-ORDER` — stable
- `G-LET-TUPLE` — stable
- `G-MAP-BLOCK-AMBIG` — stable
- `G-MATCH-ARM-EXPR` — stable
- `G-PAT-BIND` — stable
- `G-PIPE` — stable
- `G-PLACE` — stable
- `G-STMT-LAST` — stable
- `G-STMT-TERM` — stable
- `G-STRUCT-AMBIG` — stable
- `G-STRUCT-NONEMPTY` — stable
- `G-STRUCT-UPDATE` — stable
- `G-TYPE-FALLIBLE` — stable
- `G-TYPE-NOT-QUALIFIED` — stable
- `G-TYPE-UNIT` — stable
- `L-BOOL` — stable
- `L-COMMENT-BLOCK` — stable
- `L-COMMENT-LINE` — stable
- `L-CONTEXTUAL` — stable
- `L-DISPLAY` — stable
- `L-ENCODING` — stable
- `L-ESCAPE` — stable
- `L-FLOAT` — stable
- `L-IDENT` — stable
- `L-INT` — stable
- `L-INTERP` — stable
- `L-INTERP-ONE` — stable
- `L-KEYWORD` — stable
- `L-LIB-AS-I64` — stable
- `L-LIB-CMP` — stable
- `L-LIB-EXIT` — stable
- `L-LIB-FIXED` — stable
- `L-LIB-FREE` — stable
- `L-LIB-IO` — stable
- `L-LIB-LIST` — stable
- `L-LIB-MAP` — stable
- `L-LIB-MATH` — stable
- `L-LIB-NUM` — stable
- `L-LIB-OPTION` — stable
- `L-LIB-PAIRS` — stable
- `L-LIB-ROUND` — stable
- `L-LIB-SORT-BY` — stable
- `L-LIB-SORT-NAN` — stable
- `L-LIB-TO-STR` — stable
- `L-LIB-TRIM-SIDES` — stable
- `L-LIB-TUPLE` — stable
- `L-LIB-WORDS` — stable
- `L-MAXMUNCH` — stable
- `L-NEWLINE` — stable
- `L-RAW-STRING` — stable
- `L-STRING` — stable
- `L-WHITESPACE` — stable
- `T-ARM-TYPES` — stable
- `T-CAPTURE-WRITE` — stable
- `T-COMPARE` — stable
- `T-CONST-PUB` — stable
- `T-COPY` — stable
- `T-DIVERGE` — stable
- `T-EFFECT-DECL` — stable
- `T-EFFECT-DISCHARGE` — stable
- `T-EFFECT-FALLIBLE` — stable
- `T-EFFECT-FALLIBLE-CONSUME` — stable
- `T-ERR-SUBSET` — stable
- `T-FAIL` — stable
- `T-FALLIBLE` — stable
- `T-FALLIBLE-COALESCE` — stable
- `T-FALLIBLE-UNHANDLED` — stable
- `T-FALLIBLE-UNIT` — stable
- `T-FALLIBLE-VALUE` — stable
- `T-FIELD-CALL` — stable
- `T-FN-VALUE` — stable
- `T-GENERIC-FN` — stable
- `T-GENERIC-INST` — stable
- `T-INFER-LOCAL` — stable
- `T-INOUT-EXCLUSIVE` — stable
- `T-INOUT-MUTABLE` — stable
- `T-INOUT-PLACE` — stable
- `T-INOUT-REF` — stable
- `T-INT-TO-FLOAT` — stable
- `T-INT-TO-FLOAT-CARRIED` — stable
- `T-LET-IMMUTABLE` — stable
- `T-LIST` — stable
- `T-MAP` — stable
- `T-MAP-KEY` — stable
- `T-MATCH-ALT` — stable
- `T-MATCH-EXHAUSTIVE` — stable
- `T-MATCH-GUARD` — stable
- `T-MATCH-NESTED` — stable
- `T-MATCH-NESTED-COVER` — stable
- `T-MODULE-DEP` — stable
- `T-MODULE-NS` — stable
- `T-MODULE-PUB` — stable
- `T-MODULE-RESOLVE` — stable
- `T-MOVE-LOOP` — stable
- `T-MUTATE-PLACE` — stable
- `T-NO-NULL` — stable
- `T-OK-ERR` — stable
- `T-OPTION` — stable
- `T-PARAM-INOUT` — stable
- `T-PARAM-INOUT-PLACES` — stable
- `T-PARAM-LET` — stable
- `T-PARAM-OWN` — stable
- `T-PRIM` — stable
- `T-RECURSIVE` — stable
- `T-REF-EQ` — stable
- `T-REF-MATCH` — stable
- `T-REF-STALE` — stable
- `T-REGION` — stable
- `T-REGION-ESCAPE` — stable
- `T-REGION-INOUT` — stable
- `T-REGION-REF` — stable
- `T-STRUCT` — stable
- `T-SUM` — stable
- `T-TRY` — stable
- `T-TUPLE` — stable
- `T-TYPE-NO-PUB` — stable
- `T-UNIT-RETURN` — stable
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

- `E-CLOSURE-CAPTURE` — provisional
- `E-CONST` — provisional
- `E-HANDLER-MISSING` — provisional
- `E-HANDLER-OWN-ROW` — provisional
- `E-RECURSION-DEPTH` — provisional
- `G-NEST-LIMIT` — provisional
- `L-LIB-CLOCK` — provisional
- `L-LIB-CODES` — provisional
- `L-LIB-PARSE` — provisional
- `L-LIB-REMOVE` — provisional
- `L-LIB-STR` — provisional
- `L-UFCS` — provisional
- `T-EFFECT-ROW` — provisional
- `T-EMPTY-LIT` — provisional
- `T-FN-TYPE` — provisional
- `T-GENERIC-TYPE` — provisional
- `T-INT-ALIAS` — provisional
- `T-MAP-KEY-GENERIC` — provisional
- `T-OK-ERR-RETURN` — provisional
- `T-REF-CAPTURE` — provisional
- `T-REF-GENERATIONAL` — provisional
- `T-REF-VALUE` — provisional
- `T-REF-WRITE` — provisional
- `T-REGION-FOREIGN` — provisional
- `T-TYPE-ALIAS` — provisional
- `T-UNSAFE-FFI` — provisional

## Experimental

May change or be removed in any release. New surface starts here.

- `E-ACTOR` — experimental
- `E-BENCH-WARMUP` — experimental
- `E-CHANNEL` — experimental
- `E-CONCURRENT` — experimental
- `E-DEADLOCK` — experimental
- `E-IF-FALLIBLE-MERGE` — experimental
- `E-NOT-PARALLEL` — experimental
- `E-PAR-FAIL` — experimental
- `E-PAR-FILTER-ORDER` — experimental
- `E-PAR-MAP-ORDER` — experimental
- `E-PAR-VALUES` — experimental
- `E-PARALLEL-JOIN` — experimental
- `E-SCHEDULE` — experimental
- `E-SPAWN-ORDER` — experimental
- `E-SPAWN-SHARED` — experimental
- `E-TASK-PANIC` — experimental
- `E-TEST-PROPERTY` — experimental
- `E-TEST-SEED` — experimental
- `E-TEST-SHRINK` — experimental
- `G-BENCH-CONTEXTUAL` — experimental
- `G-TEST-PARAMS` — experimental
- `G-WILDCARD-BINDING` — experimental
- `L-BUNDLE-SOURCE` — experimental
- `L-LEADING-DOT` — experimental
- `L-LIB-BASE` — experimental
- `L-LIB-BITS` — experimental
- `L-LIB-BYTES` — experimental
- `L-LIB-EPRINT` — experimental
- `L-LIB-FS` — experimental
- `L-LIB-NET` — experimental
- `L-LIB-NET-PEER` — experimental
- `L-LIB-PROC` — experimental
- `L-LIB-RANDOM` — experimental
- `L-LIB-RANGE-OPS` — experimental
- `L-LIB-SUM` — experimental
- `L-LIB-UDP` — experimental
- `T-ALIAS-NOT-RECURSIVE` — experimental
- `T-CAPABILITY` — experimental
- `T-CONTRACT-ENSURES` — experimental
- `T-CONTRACT-PURE` — experimental
- `T-CONTRACT-REQUIRES` — experimental
- `T-EFFECT-BUILTIN` — experimental
- `T-EFFECT-OP-RESULT` — experimental
- `T-EFFECT-RESERVED` — experimental
- `T-ERR-UNION-OPAQUE` — experimental
- `T-EXTERN` — experimental
- `T-EXTERN-POINTER` — experimental
- `T-EXTERN-VALUE` — experimental
- `T-FN-TYPE-ROW` — experimental
- `T-FN-TYPE-ROW-CARRIED` — experimental
- `T-FN-TYPE-ROW-PARAM` — experimental
- `T-FN-VALUE-ROW` — experimental
- `T-INTERFACE` — experimental
- `T-INTERFACE-BOUND` — experimental
- `T-INTERFACE-IMPL` — experimental
- `T-INTERFACE-PURE` — experimental
- `T-INTERFACE-UNSATISFIED` — experimental
- `T-LINEAR` — experimental
- `T-LINEAR-CONTAIN` — experimental
- `T-PAR-MAP-PURE` — experimental
- `T-TASK-CAPTURE` — experimental
- `T-UNSAFE-UFCS` — experimental
