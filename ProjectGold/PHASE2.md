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

Those four decide the whole phase.

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

### Writable but degraded (3)

| # | Project | What it loses |
|---|---|---|
| 6 | Collaborative Todo | **F13/F14.** No WebSockets and no SSE, because both need a connection held open while others are served. Degrades to polling — which works, and is honestly worse. |
| 10 | Weather Mashup | `tcp_connect` can dial, but there is no TLS, so it can fetch `http://` and not `https://` — which is most public APIs. Degrades to a local stand-in service. |
| 13 | Real-Time Analytics | high-frequency streaming, one blocking connection at a time. Degrades to batched POSTs. |

### Blocked (3)

| # | Project | On what |
|---|---|---|
| 8 | File Upload Gallery | **F9.** `multipart/form-data` is binary and a socket carries `str`. |
| 9 | Chat Room Router | **F13.** Many simultaneous connections; the blocking accept serves one. |
| 11 | E-Commerce Cart | **F14.** The project *is* race conditions and state synchronisation, and the language cannot express concurrent access to shared state — which is either a cheat or the point, depending on how honest the write-up is. |

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

A "full-stack" claim usually implies serving many users at once. Grasm can serve
**one connection at a time**, and can compute in parallel only when no I/O is
involved. Every project below is therefore a correct single-user application and
none of them is a concurrent server.

That is not a reason to skip the phase — a single-user full stack in one
language, with the same engine on both sides and capabilities enforced across
the boundary, is a real thing to have shown. It is a reason not to describe it
as something it is not.
