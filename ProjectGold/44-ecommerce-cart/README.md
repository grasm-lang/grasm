# 44 — E-Commerce Cart  *(phase 2, project 11)*

**Tests:** a cart, an inventory, and the race the exercise is actually about.
**Result:** works. **This project was listed as blocked** in the phase 2 plan,
and the reason it was listed as blocked turned out to be wrong.

```
grasm run main.gr              # the comparison the harness makes
grasm run main.gr -- serve     # then serve index.html over http
```

## What the plan said

> **11** | E-Commerce Cart | **F14.** The project *is* race conditions and state
> synchronisation, and the language cannot express concurrent access to shared
> state — which is either a cheat or the point, depending on how honest the
> write-up is.

The easy version of that write-up is: *one thread, so no data race, so the
exercise is free*. That is true and useless.

**It is also wrong.** One thread removes *data* races — two cores writing one
word. It does not remove **check-then-act** races, and `net_wait` is exactly the
thing that makes them reachable: the server can serve somebody else's request in
the gap between reading the stock and writing it back.

The last unit gets sold twice, on one core, with no locks anywhere and nothing
a mutex would have fixed.

## The bug, on one thread

```
--- check, yield, act: the last unit sold twice ---
  stock before: 1
  ada checks: may take = true
  bob checks:  may take = true   <- while ada is half-done
  stock after: -1  <- negative: sold twice
  sold: 2 of 1
```

Neither `may_take` nor `take_unchecked` is wrong on its own. The wrongness is
entirely in the gap between them, and the gap is a place `net_wait` can put
somebody else's request.

## The fix, and what it is not

```
--- check and take together: it cannot happen ---
  ada takes: true
  bob takes: false   <- refused, and the check is the same instruction as the write
  stock after: 0
  sold: 1 of 1
```

This is **not a lock** and does not need to be. On one thread the gap is the
only hazard, and a function with no gap in it has none.

That is a property of *this code* rather than of the language, and it stops
being enough the moment two cores run it. Which is the honest statement of F14:
not "the exercise is free" but "the exercise is half of itself". The half about
interleaving is real and demonstrated; the half about memory ordering cannot be
written here at all.

## Over real sockets

Both carts hold the last signed spec, both checkouts go on the wire before
either is served, and the server has two genuinely pending checkouts for one
item:

```
  2 checkouts in flight before either was served
  1 bought it, 1 were told it was gone
  stock after: 0, sold 1
  never negative, and never sold twice: true
```

The harness reports **statuses, not bodies** — which of the two won depends on
the order the kernel made them ready, and that is not a fact this program may
print.

## Three smaller decisions

**A cart reserves nothing.** Adding three mugs leaves three mugs on the shelf;
stock is decided at checkout, which is the only moment it can be decided
correctly. A cart that reserved stock would let anyone empty the shop by filling
a cart and walking away.

**A partial checkout is reported, not rolled back.** Telling somebody they got
one of two things is better than telling them they got nothing — 409 with
`bought` and `unavailable` both listed.

**There is no route that uses the unsafe pair.** `may_take` and
`take_unchecked` exist to be demonstrated, not served. A shop that shipped the
broken path behind a flag would eventually have somebody set the flag.

## The honest limits

* **F14 is not closed.** This is concurrency, not parallelism. On many cores the
  same code needs real locks, and `par_map` still refuses every effect so the
  many-cores version cannot be written at all.
* **Anyone can claim to be anyone.** `X-Customer: ada` is a header, not a
  session. Project 37 has the token that should be here; wiring them together is
  left undone rather than half-done.
* **Nothing is persisted**, and prices are integer cents — which is right, and
  is the one piece of money handling this gets to be smug about.
