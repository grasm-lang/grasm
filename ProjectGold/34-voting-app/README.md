# 34 — Dynamic Voting App  *(phase 2, project 4)*

**Tests:** JSON in and out, a shared module, and a ballot box that survives
concurrent voters.
**Result:** works.

## The first project made of more than one file

Thirty-three projects before this one were a single `main.gr`. This one imports
**project 13's parser, extracted into a package** so a second project can use it
rather than carry a second copy.

What a module exports is the one decision extraction forces you to make. The
answer here is `parse_json`, `to_text`, `equal`, `escape_json`, `number_text`
and the `Json` type — the scanner state, the character cursor and the
sub-parsers stay private, because nothing outside needs to know a recursive
descent parser is what is behind the door.

All three engines agree on it, which is the thing worth checking: `grasm run`,
the bundle and `--native` each resolve the import and produce identical output.

It started as a **copy** of project 13's code. Project 35 needed the same
parser, and two copies is where a shared library stops being a nicety — so it is
a package now, `../lib-json`, and both projects import the same file. This
project's output did not change by a byte when it moved.

The reason it could not simply be a sibling file belongs here rather than in a
footnote: Grasm resolves `import json` as a sibling of the importing file, so
sharing across directories needs a package with a `grasm.toml` and a path
dependency. That is what project 5 was scheduled to prove, and it did.

## JSON in, JSON out — but not symmetrically

Votes arrive as JSON and are **parsed** by the module. The poll is **rendered by
hand**, and that asymmetry is deliberate: rendering three fields is a fold, and
building a `Json` tree in order to flatten it immediately would be work in the
shape of a demonstration rather than a program.

Every way a vote can fail has its own message, because "bad request" tells a
caller nothing it can act on:

```
not JSON               -> 400 {"error": "not JSON: expected `u` but found `o` at character 1"}
not an object          -> 400 {"error": "expected an object, got [\"native\"]"}
no "option" key        -> 400 {"error": "no \"option\" key"}
"option" is a number   -> 400 {"error": "\"option\" must be a string, got 3"}
option not on ballot   -> 422 {"error": "no such option: wasm"}
```

A vote for something never offered is a **refusal**, not a new option — 422
rather than 400, because the request was well-formed and the answer is still no.

## What the exchange asserts

Seven ballots go out before any is counted. As in project 32, what is printed is
what does not depend on the order the server got to them.

A status is a property of one request. The tallies *inside each reply* are not —
a voter sees a different running total depending on when it was counted. The
**final** tallies are, because addition commutes, and that is exactly the
invariant a ballot box must have: seven ballots in, the five valid ones counted,
whatever order they arrived in.

## The honest limits

* **One thread.** Concurrency, not parallelism — the same limit as project 32.
* **No lost update, and again that is not an achievement.** One thread touches
  the tallies. Project 11 is where that stops being free.
* **The stored file holds tallies, not the ballot.** The question and the
  options come from the program. A stored poll whose options no longer match
  the code is a poll nobody can read, so the file is deliberately the smaller
  half.

## Verified live

```
$ curl -s localhost:8975/poll
{"question": "Which engine is normative?", "options": [...], "total": 5}
$ curl -s -XPOST localhost:8975/vote -d '{"option":"native"}'   -> 200, total 6
$ curl -s -XPOST localhost:8975/vote -d '{"option":"wasm"}'     -> 422 no such option
$ curl -s -XPOST localhost:8975/vote -d 'nope'                  -> 400 not JSON
$ curl -s -XPOST localhost:8975/reset                           -> 200, all zero
```
