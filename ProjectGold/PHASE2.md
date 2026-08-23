# Phase 2 — full-stack, and what phase 1 says about it

Thirteen full-stack projects. The premise is that Grasm compiles to both a
backend and something a browser runs, and **both halves now exist**:
`grasm_wasm` puts the reference interpreter in a tab (`docs/wasm.md`), and
`net` plus `tcp_connect` make a server and a client.

So this phase is not blocked on the *stack*. It is blocked, in places, on four
specific things phase 1 found — and the honest thing is to say which projects
those are before writing any of them, rather than discovering it thirteen times.

## What phase 1 established

| | |
|---|---|
| **works** | HTTP request parsing and response framing (21), a live server, a live client (22), JSON parse and serialise (13), a hash table (16), SHA-256 (27), a SQL-ish query engine (28), markdown (12), file round-trips (11) |
| **F14** | no concurrent I/O: `par_map` refuses every effect, `spawn` is single-core |
| **F13** | every socket call blocks; no readiness check, no timeout |
| **F9** | no byte type: a socket and a file both carry `str` |
| **F7** | no stderr |

Those four decided the whole phase **when this file was written**, and three of
them have since been closed. Kept above as they were, and corrected below,
because a plan that quietly rewrites its own premises is a plan nobody can
check.

| | now |
|---|---|
| **F13** | **closed.** `net_wait` ([L-LIB-NET]) answers with the handles that can be read without blocking, so one thread holds many conversations. |
| **F9** | **closed.** `bytes` exists ([L-LIB-BYTES]), and as of this phase `tcp_read_bytes` / `tcp_write_bytes` let a socket speak it. |
| **F7** | **closed.** `eprint` ([L-LIB-EPRINT]). |
| **F14** | **half closed.** `par_map` and `par_filter` are multi-core over boxed values now, so *parsing* on many cores works. They still refuse every effect, so *fetching* on many cores does not. |

## The thirteen, classified before starting

### Writable today (7)

| # | Project | Why it works |
|---|---|---|
| 1 | Full-Stack Counter | one request at a time; state in a file (11) |
| 2 | User Registration Form | validation both sides; `write_file` for the store |
| 3 | Live Markdown Editor | 12 is already the compiler; POST text, return HTML |
| 4 | Dynamic Voting App | JSON in and out; 13 is already the parser |
| 5 | Simple Blog Dashboard | server-rendered *and* client-rendered, compared |
| 7 | User Authentication | **27's SHA-256 is the password hash** — the one place a from-scratch crypto engine stops being a demo |
| 12 | Custom SPA Router | history API in the browser; wildcard routing on the server |

Each is one `server.gr`, one `index.html`, and a `grasm.wasm` where the browser
half needs Grasm rather than JavaScript.

### Writable but degraded (1, was 3)

| # | Project | What it loses |
|---|---|---|
| 10 | Weather Mashup | `tcp_connect` can dial, but there is no TLS, so it can fetch `http://` and not `https://` — which is most public APIs. Degrades to a local stand-in service. **Unchanged: TLS is still absent.** |

`6` and `13` were listed here because holding a connection open while serving
others was impossible. `net_wait` made it possible, so **server-sent events are
writable** and neither has to degrade to polling. Both are written — project 40, two
watchers held open while three edits are pushed to both, and project 42, which
coalesces twelve events into three pushes.

### Written so far

| plan # | directory | note |
|---|---|---|
| 1 | 32-fullstack-counter | the shape the rest follow |
| 2 | 33-user-registration | one copy of the rules, via wasm |
| 3 | 36-markdown-editor | found and closed F33 |
| 4 | 34-voting-app | first multi-file project |
| 5 | 35-blog-dashboard | `lib_json` became a package |
| 7 | 37-user-auth | found F34 (no randomness) |
| 8 | 38-upload-gallery | was blocked; `bytes` over TCP unblocked it |
| 9 | 39-chat-router | was blocked; `net_wait` unblocked it |
| 6 | 40-collaborative-todo | was "degraded to polling"; it is not |
| 12 | 41-spa-router | one matcher, browser and server |
| 13 | 42-realtime-analytics | was "degraded to batched POSTs"; it coalesces instead |

### Blocked (1, was 3)

| # | Project | On what |
|---|---|---|
| 11 | E-Commerce Cart | **F14.** The project *is* race conditions and state synchronisation. `net_wait` gives many conversations on one thread, which makes the *interleaving* real and the shared state safe by construction — there is only one thread to race. That is a correct application and a dishonest demonstration of the thing the exercise is about, and the README has to say which it is. |

`8` was blocked on F9 and is **written** — project 38. `tcp_read_bytes` reads a
`multipart/form-data` body without turning the JPEG into U+FFFD, and the project
prints the difference on every run: ten bytes become sixteen through a `str`.
`9` was blocked on F13 and is **written** — project 39, which holds three
connections open while three other people post.

## The order, and why

1. **1, 2, 4** first — they establish the shape (server.gr + index.html + wasm)
   and share almost everything.
2. **3 and 5** next — they reuse 12 and 13 wholesale, so they test *module
   reuse* across projects, which nothing in phase 1 did.
3. **7** — the one that matters. A real password hash, a real session token, a
   guarded route. It is where SHA-256 stops being an exercise, and where
   `[T-CAPABILITY]` gets a demonstration with stakes: a package that handles
   passwords and is refused `net`.
4. **12** — the SPA router, which is mostly browser work.
5. **6, 10, 13** — the degraded three, each with its degradation stated in its
   own README rather than glossed.
6. **8, 9, 11** — written up as blocked, with the same standard as phase 1's
   four: what it needs, what would unblock it, and what part *is* writable.

## The honest headline, before any of it is written

*Written before phase 2 began, and now half wrong. Both versions are here.*

**Then:** a "full-stack" claim usually implies serving many users at once.
Grasm can serve **one connection at a time**, and can compute in parallel only
when no I/O is involved. Every project below is therefore a correct single-user
application and none of them is a concurrent server.

**Now:** `net_wait` means a Grasm server holds many connections at once and
answers whichever is ready, on one thread — which is what a concurrent server
is, and what most of them are underneath. What is still true is the other half:
the I/O and the computation cannot be spread across cores at the same time,
because `par_map` refuses effects. So these are concurrent single-core servers,
which is a smaller claim than "full-stack" usually carries and a much larger one
than the paragraph above allowed.

That is not a reason to skip the phase — a single-user full stack in one
language, with the same engine on both sides and capabilities enforced across
the boundary, is a real thing to have shown. It is a reason not to describe it
as something it is not.
