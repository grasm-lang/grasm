# 28 — SQL Query Engine Prototype
**Tests:** query parsing, typed columns, filtering, ordering, aggregation, NULL. **Result:** works, all three engines agree.

## What it exercises

The list asks for `SELECT name FROM users WHERE id = 1`, which `split(" ")`
handles and which tests nothing. So this parses a grammar: projection lists,
`WHERE` with `AND`/`OR` and six comparison operators, `ORDER BY ... ASC|DESC`,
`LIMIT`, and five aggregates — over **typed cells**, so `WHERE age > 30` compares
numbers and `WHERE name < 'c'` compares text and the engine knows which is which.

`AND` binds tighter than `OR`, checked with a query that gives the wrong answer
under either other reading.

## NULL is the real content

Three-valued logic is where every hand-written query engine is wrong, so it gets
its own section. A row is returned only when the predicate is **definitely
true** — `Unknown` is not a kind of true:

* `WHERE age = NULL` → no rows. Not the row where age is NULL.
* `WHERE age != NULL` → no rows either.
* `WHERE age > 20 AND city = NULL` → no rows.
* `WHERE age > 20 OR city = NULL` → the rows where age > 20, because `True OR
  Unknown` is `True`.
* `COUNT(age)` is 4 where `COUNT(*)` is 5 — aggregates skip NULL, which is the
  SQL rule people are most often surprised by.

One bug found and fixed here: `ORDER BY age DESC` scattered the NULL row into the
middle of the results, because every comparison involving one is unknown, so a
NULL row never displaced anything and nothing displaced it either. NULLs now sort
together at the end in both directions.

Eleven malformed queries, each an error with a reason.

## Findings
None in the language. F1 was hit again while writing the tokenizer, and the
improved diagnostic (added the same day) named the fix.
