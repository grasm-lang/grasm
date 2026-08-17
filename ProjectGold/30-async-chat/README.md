# 30 — Fully Async Chat Server & Client — **blocked**

**Tests:** non-blocking I/O, epoll/kqueue polling, connection lifecycles.
**Result:** cannot be written today.

## Why

**Every socket operation blocks** (FINDINGS.md F13). `tcp_accept` waits for a
connection and `tcp_read` waits for bytes; there is no `select`, no poll, no
timeout, and no way to ask whether a handle is ready.

So a single-threaded Grasm server serves exactly one connection at a time. That
is what project 21 does, deliberately and correctly — and a chat room where the
second person has to wait for the first to disconnect is not a chat room.

The client half is blocked separately by F11: no `tcp_connect`.

## Why this one is different from the others

Every other blocked project needs a *builtin*. This one needs a **design
decision**, and it reaches into the part of the language that is supposed to be
its differentiator.

The effect vocabulary already contains `async` and nothing performs it. Grasm's
whole thesis is that effects are tracked in the type system, so the shape of the
answer is presumably that a blocking read performs `async` and a handler decides
what that means — which is exactly the structure WebAssembly's stack-switching
proposal is being specified with, and exactly what `MASTERPLAN` §19 gestures at.

That is a real piece of language design, not a missing function, and it is the
largest single thing standing between Grasm and the "structured, uncolored
concurrency" its own north-star goal G6 claims.
