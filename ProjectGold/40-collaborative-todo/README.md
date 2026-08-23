# 40 — Collaborative Todo  *(phase 2, project 6)*

**Tests:** shared state behind a held connection — two people editing one list,
each seeing the other's edits as they happen.
**Result:** works. The plan said it would **degrade to polling**; it does not.

```
grasm run main.gr              # the comparison the harness makes
grasm run main.gr -- serve     # then open index.html in two tabs
```

## What the plan expected

> **F13/F14.** No WebSockets and no SSE, because both need a connection held
> open while others are served. Degrades to polling — which works, and is
> honestly worse.

`net_wait` closed F13. Project 39 showed a connection held open while others are
served; this one puts shared state behind it. What makes it *collaborative*
rather than merely multi-user is that a change made by one client is **pushed**
to the others — polling would reach the same final list and a worse experience,
and the difference is exactly the connection the language could not hold.

## The demonstration

Two watchers are held open, then three edits arrive from other clients. Each
edit is pushed to both watchers as it happens — before the editor is even told
its own edit landed.

```
  2 watchers held open
  3 edit(s) accepted
  watcher 0: 4 push(es) — one hello and one per edit
    first: data: {"todos": [], "open": 0}
    last:  data: {"todos": [{"id": 1, ..., "done": true}, {"id": 2, ...}], "open": 1}
  watcher 1: 4 push(es) — the same four
  stored:    the same list again
```

Both watchers and the store agree, which is the property a collaborative list
has to have.

## Two decisions worth their comments

**A subscriber is sent the state immediately, then nothing until something
changes.** Without that first push a tab would show an empty list until somebody
else happened to edit.

**Events carry the whole list, not the change.** A diff would require every
client to already hold the same list, and a client that missed one event would
be quietly wrong from then on. Sending state makes a dropped event cost latency
rather than correctness — and it is why the browser does no optimistic update
and no re-fetch: the stream is the only thing that moves the list, so every tab,
including the one that made the edit, learns the same way.

**An event is one event.** A newline in a todo's text would end it early and let
one todo forge the events after it. The harness sends
`a\nevent: state\ndata: forged` and shows it arriving as a single line.

## Verified live

Two `curl -N` connections held open across two edits:

```
$ curl -N localhost:8981/events &   # x2, both held
$ curl -XPOST localhost:8981/todos -d 'text=live+edit'   201
$ curl -XPOST localhost:8981/todos/1/toggle              200

watcher A: 3 pushes, last {"todos": [{"id": 1, "text": "live edit", "done": true}], "open": 0}
watcher B: 3 pushes, last — identical
```

## The honest limits

* **Last write wins.** Two people editing the same todo at the same instant: the
  second overwrites the first, silently. Real collaborative editing needs
  versions or a CRDT, and this has neither.
* **One thread.** Many conversations, one core.
* **No identity.** Anyone who can reach the port can edit. Project 37 is the one
  with authentication; wiring the two together is left undone rather than
  half-done.
* **No history.** A watcher joining later sees the current list and never learns
  what changed before it arrived — which is fine here precisely *because*
  events carry state rather than diffs.
