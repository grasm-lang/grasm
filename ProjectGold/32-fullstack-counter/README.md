# 32 — Full-Stack Counter  *(phase 2, project 1)*

**Tests:** HTTP over real sockets, concurrent connections on one thread, state
that outlives the process, and a browser talking to a Grasm server.
**Result:** works.

The smallest thing that is honestly full-stack, and the first project of phase
2 — so it exists as much to establish the shape the other twelve follow as to
count anything.

## The three halves

| | what it is | checked how |
|---|---|---|
| **core** | parsing a request, applying an action, rendering JSON | pure, so it runs on every engine on every run and is compared |
| **exchange** | five clients and one server over real sockets, in one process | deterministic, so it is compared too |
| **live** | `-- serve`, plus `index.html` | needs a browser; run by hand |

```
grasm run main.gr              # the comparison the harness makes
grasm run main.gr -- serve     # then open index.html
```

## What the exchange demonstrates

Five requests go out **before any reply is read**, so the fifth is on the wire
while the first is still being answered, and one `net_wait` loop drives both
halves. A blocking `tcp_accept` here would deadlock this program against
itself on the first dial — which is why this could not be written before
[L-LIB-NET] grew readiness, and why phase 1 recorded it as **F13**.

## What it reports, and what it deliberately does not

Statuses are printed; **values are not**. A status is a property of one request
— `/nowhere` is 404 whenever it is served — but the value in a reply is not:
three clients sending `+4`, `+1` and `-2` to one counter see `{4, 5, 3}` in one
service order and `{1, 5, 3}` in another, and both are correct.

Printing them would make this program disagree with itself between runs, and a
test that is flaky about the very thing it demonstrates is worse than no test.
What *is* asserted is the final value, because addition does not care about
order: five concurrent clients, one counter, **no update lost**.

## The honest limits

* **One thread.** `net_wait` gives concurrency, not parallelism. Requests
  interleave; they do not run at the same instant. That is what most servers do
  underneath, and it is not what "scales across cores" means.
* **No update can be lost, and that is not an achievement.** There is exactly
  one thread touching the counter, so the data race this kind of demo usually
  has to defend against cannot be written. Phase 2's project 11 is the one where
  that stops being a free win and starts being **F14**.
* **The browser half is JavaScript**, not Grasm compiled to wasm. There is
  nothing here for Grasm to compute that `fetch` does not already do, and
  reaching for the wasm build to send an HTTP request would be a demonstration
  of nothing. Projects 3 and 12 are where the browser half has real work.
* **State is a text file holding one integer.** This project is about the
  stack. Project 4 is the one about JSON.

## Verified live

Against a state file left at 3 by an earlier run, which is the point of the
state file:

```
$ curl -s localhost:8973/count              -> {"value": 3, "action": "get"}
$ curl -s -XPOST localhost:8973/inc -d n=5  -> {"value": 8, "action": "inc"}
$ curl -s localhost:8973/ | head -c 60      -> <!doctype html><meta charset="utf-8"><title>Counter</title><
$ curl -s -XPOST localhost:8973/reset       -> {"value": 0, "action": "reset"}
```

The server prints where that file is on startup, so a run can be reset by
deleting it.
