# ProjectGold — phase 2 results

Thirteen full-stack projects: a browser, an HTTP server, and state that outlives
the process. **All thirteen are written and agree byte for byte across all three
engines** — `grasm run`, `grasm build`, and `grasm build --native`. **Nothing is
blocked.**

Re-run with `./run.sh` (44 projects across both phases).

| plan # | directory | Status | What it found |
|---|---|---|---|
| 1 | 32-fullstack-counter | ✅ | the shape the rest follow |
| 2 | 33-user-registration | ✅ | **F32** the committed wasm was four months behind the language |
| 3 | 36-markdown-editor | ✅ | **F33** the wasm engine could not run a program with an `import` |
| 4 | 34-voting-app | ✅ | first multi-file project |
| 5 | 35-blog-dashboard | ✅ | `lib_json` became a package; two consumers, one file |
| 6 | 40-collaborative-todo | ✅ | was "degrades to polling"; it does not |
| 7 | 37-user-auth | ✅ | **F34** there is no source of randomness |
| 8 | 38-upload-gallery | ✅ | was **blocked**; `tcp_read_bytes` unblocked it |
| 9 | 39-chat-router | ✅ | was **blocked**; `net_wait` unblocked it |
| 10 | 43-weather-mashup | ✅ | still degraded: **no TLS**, and nothing here fixes it |
| 11 | 44-ecommerce-cart | ✅ | was **blocked**, and the reason was wrong |
| 12 | 41-spa-router | ✅ | one route matcher, browser and server |
| 13 | 42-realtime-analytics | ✅ | was "degrades to batched POSTs"; it coalesces |

## The plan predicted three blocked and three degraded. It got two right.

| | predicted | actual |
|---|---|---|
| 8 File Upload | blocked on F9 | **writable** — `bytes` existed, the socket had no way to speak it |
| 9 Chat Router | blocked on F13 | **writable** — `net_wait` |
| 11 Cart | blocked on F14 | **writable**, and the reasoning was wrong |
| 6 Collaborative Todo | degrades to polling | **no degradation** |
| 13 Analytics | degrades to batched POSTs | **no degradation** |
| 10 Weather Mashup | degrades: no TLS | **correct** |

The plan was written when four findings decided the phase, and three of them
closed before the phase began — F13 by `net_wait`, F7 by `eprint`, F9 by `bytes`
plus the byte socket this phase added. Only the TLS prediction survived contact.

**Project 11's prediction was wrong for an interesting reason.** It said one
thread makes shared state safe by construction, so a cart would be a correct
application and a dishonest demonstration. One thread removes *data* races and
not **check-then-act** races, and `net_wait` is what makes those reachable — the
server can serve another request in the gap between reading the stock and
writing it back. The project oversells the last unit on one core with no locks
anywhere, and then does not. Half the exercise was writable all along and nobody
had checked.

## What the phase changed about the language

| | |
|---|---|
| `tcp_read_bytes`, `tcp_write_bytes` | a socket can carry a JPEG. Ten bytes became sixteen through a `str` |
| `grasm build --source` | a whole program as one `.gr`, so the wasm engine can run one with imports |
| `scripts/wasmtest.sh` | now compares every committed `grasm.wasm` against a fresh build, and knows what wasm32 cannot do |
| `lib_json`, `lib_markdown`, `lib_sha256` | three packages extracted from phase 1, each with `capabilities = []`. `lib_json` has since moved to `packages/json` and been renamed `json`, as the release library; the six consumers here point at it. |

## What it did not change, and named instead

* **No TLS.** `http://` and not `https://`. The largest single thing between
  this language and ordinary web work. Project 43 says so first rather than
  last.
* **No randomness (F34).** `now_millis()` and nothing else. Survivable for a
  salt, whose job is uniqueness; not survivable for a session token, which is
  exactly a secret that must not be guessable. Project 37 issues them anyway and
  says why they are unsafe.
* **F14 is not closed.** Every project here is one thread: concurrency, not
  parallelism. `par_map` runs on many cores and refuses every effect, so a
  server that computes in parallel *and* does I/O still cannot be written.

## The honest headline

A Grasm server holds many connections at once and answers whichever is ready, on
one thread — which is what a concurrent server is, and what most of them are
underneath. What is still true is that the I/O and the computation cannot be
spread across cores at the same time.

**These are concurrent single-core servers.** That is a smaller claim than
"full-stack" usually carries, and a much larger one than the plan allowed.
