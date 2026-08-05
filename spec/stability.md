# Clause stability

The level of every normative clause in `spec/`. See `docs/stability.md` for
what each level promises.

`crates/grasm_spec` fails if a clause here is missing, unknown, or listed twice,
so this file cannot drift away from the specification it describes.


## Stable

Will not change incompatibly before 2.0.

- `E-ARGS` — stable
- `E-ARITH-FLOAT` — stable
- `E-ARITH-INT` — stable
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
- `E-NUM-PROMOTE` — stable
- `E-ORDER-LTR` — stable
- `E-OWN-RUNTIME` — stable
- `E-PANIC-EXIT` — stable
- `E-POW-NEG` — stable
- `E-SHORTCIRCUIT` — stable
- `E-STR-CHARS` — stable
- `E-STR-COMPARE` — stable
- `E-STR-CONCAT` — stable
- `E-VALUE-SEMANTICS` — stable
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
- `L-LIB-FREE` — provisional
- `L-LIB-NUM` — provisional
- `L-LIB-STR` — provisional
- `L-LIB-LIST` — provisional
- `L-LIB-MAP` — provisional
- `L-LIB-OPTION` — provisional
- `L-KEYWORD` — stable
- `L-MAXMUNCH` — stable
- `L-NEWLINE` — stable
- `L-STRING` — stable
- `L-WHITESPACE` — stable

## Provisional

Specified and tested; the design is still being validated against real
programs. May change in a minor release, with a changelog entry.

- `E-MINMAX-OPTION` — provisional
- `E-PAD-CYCLE` — provisional
- `E-SEARCH-SHORTCIRCUIT` — provisional
- `G-ARM-CONTROL` — provisional
- `G-DECL-ORDER` — provisional
- `G-NEST-LIMIT` — provisional
- `G-PAT-BIND` — provisional
- `G-PIPE` — provisional
- `G-STMT-LAST` — provisional
- `G-STMT-TERM` — provisional
- `G-STRUCT-AMBIG` — provisional
- `G-TYPE-UNIT` — provisional
- `T-ARM-TYPES` — provisional
- `T-COPY` — provisional
- `T-EFFECT-DECL` — provisional
- `T-EFFECT-DISCHARGE` — provisional
- `T-EFFECT-ROW` — provisional
- `T-EMPTY-LIT` — provisional
- `T-ERR-SUBSET` — provisional
- `T-FAIL` — provisional
- `T-FALLIBLE` — provisional
- `T-FALLIBLE-UNHANDLED` — provisional
- `T-GENERIC-FN` — provisional
- `T-GENERIC-INST` — provisional
- `T-INFER-LOCAL` — provisional
- `T-INOUT-EXCLUSIVE` — provisional
- `T-INOUT-MUTABLE` — provisional
- `T-INOUT-PLACE` — provisional
- `T-INT-ALIAS` — provisional
- `T-INT-TO-FLOAT` — provisional
- `T-LET-IMMUTABLE` — provisional
- `T-LIST` — provisional
- `T-MAP` — provisional
- `T-MAP-KEY` — provisional
- `T-MATCH-EXHAUSTIVE` — provisional
- `T-MATCH-GUARD` — provisional
- `T-MODULE-NS` — provisional
- `T-MODULE-PUB` — provisional
- `T-MOVE-LOOP` — provisional
- `T-NO-NULL` — provisional
- `T-OPTION` — provisional
- `T-PARAM-INOUT` — provisional
- `T-PARAM-LET` — provisional
- `T-PARAM-OWN` — provisional
- `T-PRIM` — provisional
- `T-STRUCT` — provisional
- `T-SUM` — provisional
- `T-TRY` — provisional
- `T-TUPLE` — provisional
- `T-USE-AFTER-MOVE` — provisional
- `T-VAR-MUTABLE` — provisional

## Experimental

May change or be removed in any release. New surface starts here.

- `E-BENCH-WARMUP` — experimental
- `E-NOT-PARALLEL` — experimental
- `E-PAR-MAP-ORDER` — experimental
- `T-FN-VALUE` — provisional
- `T-PAR-MAP-PURE` — experimental
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
- `T-TASK-CAPTURE` — experimental
