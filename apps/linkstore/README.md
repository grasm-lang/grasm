# linkstore

A link shortener, as a service meant to be deployed rather than demonstrated.
Nine libraries wired together: `http`, `http_server`, `json`, `log`,
`metrics`, `sanitize`, `uuid`, `random`, `url`.

It exists to answer a question no gate in this repository was asking. Every
gate is a program that starts, prints and exits — nothing had ever run for
more than a second or held more than a few thousand objects. So the runtime's
reference counting, which produced F79, had never been watched under sustained
churn.

## Running it

```sh
grasm build apps/linkstore/src/main.gr --native -o linkstore
./linkstore                 # the offline self-test: every route, no socket
./linkstore -- serve        # http://127.0.0.1:8080
apps/linkstore/load.sh 60   # drive it for a minute and sample its memory
```

## What it declares

`capabilities = ["io", "net", "Log", "Metric"]` — and nothing else. No
filesystem, no processes, no `unsafe`. The handler itself is a *pure function*
of the store and the request, so every route is exercised offline in the
self-test and the socket path calls the identical function.

## Routes

| | |
|---|---|
| `GET /healthz` | is the process alive |
| `GET /readyz` | is it ready for traffic |
| `GET /metrics` | Prometheus text |
| `POST /links` | create, JSON body `{"target": "https://…"}` |
| `GET /links` · `GET /links/:id` | read |
| `GET /r/:id` | redirect, and count the hit |

## What it cannot do, stated rather than implied

**No TLS.** It serves `http://` and belongs behind a proxy that terminates
TLS — which is how most services are deployed anyway.

**No background work.** Grasm's `parallel` joins before the next statement, so
there is no reaper, no eviction thread, no keep-alive sweeper. Everything
happens on a request. That is a language limitation with a name, in
`docs/packages-plan.md` under "Blocked, and on what".

**One connection at a time.** `net_wait` would allow several; this serves them
in order, which is honest for a service whose store is a list.

## What building it found

Two defects, in the first run, in this file rather than in the libraries:

**Every link got the same id.** A v7 UUID's first 48 bits *are* the
millisecond it was made — that is what makes it sort by time, and exactly what
makes the first eight hex characters identical for everything created in the
same 50-day window. `substr(0, 8)` produced one id for the whole store. It
takes from the tail now.

**A target carrying `\r\n` was stored.** `sanitize.header_value` strips control
characters on the way out, so the `Location` header was never splittable — but
the *store* held one, and the comment beside the check claimed otherwise.
Refusing at the door means no other output has to have the same guard.

Both are the kind a real developer ships. Neither was reachable by any
generator, because neither is a language defect: they are a service using a
correct library wrongly.

## What the load run measured

| | |
|---|---|
| 5,481 requests over 45s, mixed reads, writes, redirects and refusals | no crash |
| memory, read-only, 5,010 requests | grows to ~500K then **falls back** — bounded |
| memory with writes, ~783 links created | +3,280K, which is the store |
| `GRASM_LEAK_CHECK=1` on the serve path | **0 live allocations at exit** |

The read-only number is the one that matters: under sustained load with
nothing being stored, memory plateaus and comes down. That is the reference
counting holding, which is what had never been watched.
