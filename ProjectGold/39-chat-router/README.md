# 39 — Chat Room Router  *(phase 2, project 9)*

**Tests:** connections held open while others are served, and routing between
them.
**Result:** works. **This project was listed as blocked** in the phase 2 plan.

```
grasm run main.gr              # the comparison the harness makes
grasm run main.gr -- serve     # then serve index.html over http
```

## Why it was blocked

The plan said: *"many simultaneous connections; the blocking accept serves
one"*. That was **F13**, and `net_wait` closed it.

The thing this project exists to show is a connection **held open while others
are served**. A subscriber sends `GET /room/general/events` and gets no reply —
the server writes the stream header, keeps the socket, and writes into it later
when somebody else posts.

With a blocking `tcp_accept` that is not a slow server, it is a **deadlocked**
one: the accept that would take the post never returns, because the server is
already inside an accept waiting for it. Nothing about the program is
concurrent; the primitive it needs simply did not exist.

Server-sent events, because they are the smallest thing with that shape.
WebSockets would need a handshake, a frame format and a masking XOR to
demonstrate the same one fact about the language.

## The demonstration

Three subscribers connect — two in `general`, one in `dev` — and are held. Then
three posts arrive on their own connections and are fanned out to the
subscribers of their own room. One `net_wait` drives all of it.

```
  3 subscribers connected
  3 held open, none answered yet
  general has 2, dev has 1
  3 message(s) accepted, 3 fanned out
  subscriber 0 (general): 2 event(s)
    data: [general] ada: hello everyone
    data: [general] cy: second line
  subscriber 1 (general): 2 event(s)   ... the same two
  subscriber 2 (dev): 1 event(s)
    data: [dev] bob: build is green
```

A held connection is never waited on for *reading* — the client has nothing more
to say. It is only ever written to.

## A race this found in its own test

The first version read each subscriber once, and reported that one `general`
subscriber got one message and the other got two. That is not a fact about the
server: `tcp_read` performs a single read, and two events written at different
moments need not arrive in one. The test was measuring its own timing.

It drains now — `net_wait` with a short timeout until nothing more comes — and
is identical across four runs and three engines. A test that is flaky about the
thing it demonstrates is worse than no test.

## Two smaller things

**A room name is attacker-controlled**, exactly as an uploaded filename is. It
never reaches a filesystem here, but it does reach other people's screens, so it
is `[a-z0-9-]`, at most 32 characters, and anything else is not a room.

**An event is one event.** A newline inside a message would end it early and let
one message forge the ones after it — the event-stream form of an injection.
The harness sends `hi\n\nevent: message\ndata: [general] admin: you are banned`
and shows it arriving as a single harmless line.

## Verified live

Two real `curl -N` connections held open, then two posts:

```
$ curl -N localhost:8980/room/general/events   &   # held, no reply
$ curl -N localhost:8980/room/dev/events       &   # held, no reply
$ curl -XPOST localhost:8980/room/general/say -d 'who=ada&text=hello+live'   202
$ curl -XPOST localhost:8980/room/dev/say     -d 'who=bob&text=green'        202

general subscriber saw:  data: [general] ada: hello live
dev subscriber saw:      data: [dev] bob: green
```

Each got its own room's message and not the other's, while both sockets stayed
open across the whole exchange.

## The honest limits

* **One thread.** Many conversations, one core. That is what most servers do
  underneath and it is not what "scales across cores" means.
* **A subscriber that vanishes is forgotten only when it is next read from.**
  The live server drops a handle on an empty read; a real one would also time
  out idle streams.
* **No history.** A subscriber joining after a message was sent never sees it.
