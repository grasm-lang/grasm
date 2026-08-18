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

## And then it hit the real wall — half of which is now gone

**F14 — there is no way to do concurrent I/O.**

```grasm
let pages = paths.par_map(|p| fetch_one(8971, p))
```
```
error: this parallel operation needs an effect-free function, but the one given
       performs `net`
```

`par_map` is the only multicore path in the language and it refuses every
effect. `spawn` and `parallel` are deliberately single-core
([E-NOT-PARALLEL]). So a program could compute on many cores, or perform I/O,
and not both — which made the word in this project's title unwritable, and the
fetch loop was sequential and said so.

**The I/O half is fixed, and it did not need threads.** `net_wait`
([L-LIB-NET]) answers with the handles that can be read without blocking, so one
thread can hold every request open at once. `fetch_all` connects and sends
*all* requests before waiting for any response, then reads them in whatever
order they arrive and returns them in request order. That is what concurrent
fetching means in practice; threads were never the point, not blocking on the
first response was.

The default run proves it with nothing outside the program: a server and six
clients, **driven from a single wait loop**, so a request arriving and a
response arriving are the same kind of event.

```
--- six requests, one thread, no blocking ---
  6 requests in flight before any reply was read
  /: 200, 102 byte(s), title "Home", 2 link(s)
  ...
```

A blocking `tcp_accept` there would deadlock the program against itself on the
first dial, so the transcript is itself the evidence that nothing blocked.

**What is still missing is parallel *compute* alongside I/O.** `par_map` still
refuses an effectful closure, and it is right to: results would interleave
differently between runs and it promises a deterministic answer. A program that
wants to fetch on many cores *and* parse on many cores still cannot. F14 stays
open for that, and it is a smaller gap than it was.

## What is tested here

The parsing half, deterministically, on canned responses — status lines, header
and body splitting, `<title>` extraction, `href` scraping, and four malformed
inputs (a 500 with no body, a non-HTTP response, an empty one, upper-case tags).
That half runs through `par_map` as well as serially, and the results are
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
