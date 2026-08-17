# 06 — Static Contact Book

**Tests:** structs, `Map[str, T]`, `inout`, CRUD, and — the real subject —
**value semantics**.

**Result:** works. All three engines agree.

## What it actually exercises

Create, read, update, delete and search are the stated goal and are the least
interesting part. The subject is that a `Contact` is a *value*.

Grasm has value semantics, so handing a struct to a function hands it a copy, and
a function that means to modify one says `inout`. `update_email` has to take the
contact out of the map, change the copy, and put it back — there is no way to
reach into the map and mutate in place, and that is by design.

The last block is the actual test. A contact is read out, its email and its
`tags` list are both mutated, and the book is read again. The book is unchanged,
including the nested list. A language that quietly shares the struct passes
every other test in this file and fails that one.

## Findings

**F1** — a condition cannot wrap onto a continuation line that *starts* with
`or`; the operator has to end the previous line. Found here first.

Also noted: `.sort()` orders in place and returns `()`, so it is a statement and
the list is the value. The diagnostic said so clearly.
