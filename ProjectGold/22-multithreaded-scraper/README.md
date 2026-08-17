# 22 — Multithreaded Web Scraper
**Tests:** connections, concurrency, thread-safe sharing. **Result:** works —
**but not multithreaded**, and that is the finding.

## What changed

This was blocked on **F11**: `net` could listen and accept and never dial. It is
the reason `tcp_connect` was added, and the fetching half works now — verified
end to end against project 21's server, which is also a Grasm program:

```
$ grasm run main.gr -- fetch
  /: 200, 87 byte(s), title "grasm", 0 link(s)
  /nope: 404, 53 byte(s), title "", 0 link(s)
  5 of 6 returned 200
```

## And then it hit the real wall

**F14 — there is no way to do concurrent I/O.**

```grasm
let pages = paths.par_map(|p| fetch_one(8971, p))
```
```
error: this parallel operation needs an effect-free function, but the one given
       performs `net`
```

`par_map` is the only multicore path in the language and it refuses every
effect. `spawn` and `parallel` are deliberately single-core ([E-NOT-PARALLEL]).
So a program can compute on many cores, and can perform I/O, and cannot do both
at once — which makes the word in this project's title unwritable today. The
fetch loop is sequential and says so.

That is the largest gap this directory found, and it sits under the goal that
claims the most: MASTERPLAN G6 asks for "structured, uncolored concurrency", and
`docs/northstar.md` already records it as *retreated*. This is what the retreat
costs.

## What is tested here

The parsing half, deterministically, on canned responses — status lines, header
and body splitting, `<title>` extraction, `href` scraping, and four malformed
inputs (a 500 with no body, a non-HTTP response, an empty one, upper-case tags).
That half runs through `par_map` as well as sequentially, and the results are
compared: `[E-PAR-MAP-ORDER]` promises order is preserved, and a scraper whose
output changes between runs is not usable.

## The mutex that is not there

The list asks about "thread-safe memory sharing, mutexes, or lock mechanisms".
Grasm has none and needs none here, because the race cannot be written:

```
error: a `par_map` closure may not reach a `ref` declared outside it
  help: `par_map` runs its closure on several threads at once, and a `ref` is a
        handle into one shared heap
```

Results are collected by `par_map` and merged afterwards, which is the shape the
refusal pushes you into. It is a good answer to the question — it is just
answering it about pure computation, because that is the only kind of
parallelism available.
