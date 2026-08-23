# 35 — Simple Blog Dashboard  *(phase 2, project 5)*

**Tests:** the same data rendered two ways, a shared package, and a store in the
API's own format.
**Result:** works.

## Two renderings, compared rather than asserted

The server renders posts into HTML. The browser renders the same posts from
JSON. It would be easy to show that both "work" and leave it there — so this
project instead **strips the tags out of the HTML and checks the words against
the JSON**:

```
  words from html: [Blog, Blog, Three engines, The interpreter is normative...]
  words from json: [Three engines, The interpreter is normative...]
  2 word(s) of page furniture, then the posts
  the readable text agrees once the furniture is set aside: true
```

The comparison is on the *tail*, not on a fixed offset, because counting the
chrome would mean the test knew how many headings the template has — which is
exactly the thing a template is allowed to change. (It knew two, and was wrong
about one, before this was fixed.)

**Where they genuinely differ is the interesting part.** The server-rendered
page never sends the unpublished post; the API does, marked `"published":
false`, and the client-rendered page shows it greyed out as a draft. Server
rendering is a filter as well as a formatter, and that is a security property
rather than a styling one.

## The second consumer of `lib_json`

Project 34 carried a copy of project 13's parser. Two copies is where a shared
library stops being a nicety, so it is a package now — `../lib-json` — and
project 34 was converted to depend on it in the same commit that introduced this
one. Its output did not change by a byte.

That is the thing this project was scheduled to prove: `import lib_json` in two
directories means *the same file*, and all three engines resolve it.

## What the native backend made us rewrite

Two functions here were first written as nested matches — an `Err` arm that
returns beside an `Ok` arm that evaluates another match — and the native backend
refuses a `match` whose arms it cannot give one type. It says so clearly and
points at `grasm run`, but a ProjectGold project has to pass on all three
engines, so both were flattened into early returns.

The flat versions are not worse. But the refusal is real and it is recorded
here rather than smoothed over: the shape a person reaches for first is one the
whole language does not yet accept.

## The honest limits

* **`id` comes from a JSON number**, which is an `f64`, narrowed with
  `as_i64`. Anything past 2^53 stops round-tripping — a limit JSON itself has,
  not one Grasm added.
* **A post with no `id` is parsed twice**: once as sent, once with the next id
  put in front of it. Two parses of a small object is cheaper than a second
  parser that knows about optional fields, and it is stated because the second
  parse is otherwise invisible.
* **One thread**, as in every project of this phase.

## Verified live

```
$ curl -s localhost:8976/           -> <!doctype html>...<h1>Blog</h1><article>...
$ curl -s localhost:8976/api/posts  -> {"posts": [{"id": 1, ...
$ curl -s -XPOST localhost:8976/api/posts \
    -d '{"title":"Four","body":"added live","published":true}'   -> 201
$ curl -s -XPOST localhost:8976/api/posts -d 'nope'              -> 400 not JSON
```
