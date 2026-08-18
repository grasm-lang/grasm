# 30 — Fully Async Chat Server & Client — **works**

**Tests:** non-blocking I/O, readiness polling, connection lifecycles.
**Result:** one thread serves every client at once.

## What it does

Run with no arguments, the program is its own proof. It opens a listener, dials
three clients to itself, and then runs the server *and* all three clients in a
single thread:

```
--- one thread, three clients ---
dialled 3 clients
  server: accepted guest4
  ...
  server: guest4 said hello
  client1 heard guest4: hello
  client2 heard guest4: hello
```

Run with `-- serve` and it is a real chat room on port 8994, which `nc` can
join.

## Why it was blocked, and what changed

Every socket call blocked. `tcp_accept` waited for a connection, `tcp_read`
waited for bytes, and there was no way to ask whether a handle was ready
(FINDINGS.md **F13**). A single-threaded Grasm server therefore served exactly
one connection at a time — which is what project 21 does, deliberately and
correctly, and which is useless for a chat room where the second person would
have to wait for the first to hang up.

The obvious fix is a thread per connection, and Grasm cannot do that: `spawn`
does not carry the `net` capability, so a spawned task cannot touch a socket.
That is [T-CAPABILITY] working as designed, not a bug, and it means the answer
had to be readiness rather than threads.

`net_wait(handles, timeout_ms)` was added ([L-LIB-NET]). It answers with the
handles that can be read without blocking, in the order they were asked in. The
listener sits in the same wait set as the connections, so a new arrival and a
message from an existing client are the same kind of event — which is the whole
trick, and is what every real server does underneath.

## The part that could not have been demonstrated any other way

The demonstration dials its own clients *from the thread that is about to serve
them*. That only works because nothing blocks: a blocking `tcp_accept` would
deadlock the program against itself on the very first dial. The transcript is
therefore not just evidence that the server works — it is evidence that the
server never blocked.

## What it still does not have

This is readiness, not asynchrony. `tcp_read` on a ready handle still performs a
blocking read; it simply cannot block, because the data is already there. There
are no coroutines, no `async` effect being performed, and no stack switching.
**F14** — that `spawn` cannot perform I/O at all, so a program can compute in
parallel or talk to the network but not both — is untouched, and remains the
largest gap in the language.
