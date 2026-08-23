# 42 — Real-Time Analytics  *(phase 2, project 13)*

**Tests:** many collectors and many dashboards on one thread, and what a
dashboard should actually be sent.
**Result:** works. The plan said it would **degrade to batched POSTs**; it does
not.

```
grasm run main.gr              # the comparison the harness makes
grasm run main.gr -- serve     # then serve index.html over http
```

## What the plan expected

> **13** | Real-Time Analytics | high-frequency streaming, one blocking
> connection at a time. Degrades to batched POSTs.

`net_wait` holds the dashboards open while collectors keep arriving, so the
streaming half needs no compromise. The interesting engineering is the other
half.

## Pushing on every event is the wrong answer

A thousand events a second times four dashboards is four thousand writes a
second to render a number a person reads twice.

So this **coalesces**. Every event is folded into the aggregate immediately;
a push happens only once a batch has built up. The dashboards are told about
*counts*, not about *events*.

```
12 event(s) taken, 3 push(es) at a batch of 4, none left to flush
dashboard 0: 4 message(s) for 12 events
dashboard 1: 4 message(s) for 12 events   ← the same four
server total: the same numbers again
```

Live, with ten events:

```
$ curl -N localhost:8983/live &          # held open
$ for i in $(seq 10); do curl -XPOST localhost:8983/collect -d 'name=page.view&value=1'; done

events sent: 10
pushes seen: 4                            ← hello, two batches, and a flush
last push:   {"events": 10, "counters": [{"name": "page.view", "count": 10, ...}]}
```

**Coalescing is only safe because counts commute.** A dashboard that missed the
middle of a batch still ends up exactly right, because what it is sent is the
running total rather than a delta. The harness checks that directly: the same
events folded in two different orders produce identical JSON.

A partial batch is **flushed when the collectors go quiet**. Without that, the
last few events would sit unseen until the next one arrived, which on a quiet
service could be a long time — a dashboard that is right eventually and wrong
for an hour is worse than one that is slightly behind.

## Three smaller decisions

**`202`, not `200`.** The event has been taken; what a dashboard makes of it
happens later. Saying 200 would promise it had already been shown.

**The aggregate is kept and the event is not.** That is the difference between
an analytics server and a log: a log grows with traffic and this does not.

**Counters come back in sorted order**, so two servers that saw the same events
in different orders report the same thing. Without it the output depends on
which name happened to arrive first, which is not a fact anyone wants.

**The mean is an integer.** A float would be more precise and less comparable —
a dashboard showing `12.666666666666666` has told the reader nothing the integer
did not.

## The honest limits

* **The batch is a count, not a clock.** A real deployment would push "at most
  once every 250ms" as well, and this cannot do that deterministically —
  `now_millis()` would make the harness's output depend on how fast the machine
  is. The quiet-flush is the part of the clock behaviour that could be kept.
* **One thread.** Many conversations, one core.
* **Nothing is persisted.** Restart and the counters are zero. A real one writes
  the aggregate periodically, which is easy and would have added nothing this
  project is about.
* **No identity, no sampling, no retention window.** Every event counts forever.
