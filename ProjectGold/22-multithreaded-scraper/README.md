# 22 — Multithreaded Web Scraper — **blocked**

**Tests:** thread creation, thread-safe sharing, mutexes. **Result:** cannot be
written today.

## Why

Not the threads — `par_map` is real, runs on multiple cores, and project 15
already leans on it. The problem is that **a Grasm program cannot open a
connection**. [L-LIB-NET] provides `tcp_listen`, `tcp_accept`, `tcp_read`,
`tcp_write` and `tcp_close`: a program can *be* a server and cannot *be* a
client (FINDINGS.md F11).

So there is no way to fetch a URL, and the scraping half of a scraper is absent.

## What would unblock it

One builtin: `tcp_connect(host: str, port: i64) -> i64 ! IoError`. The
capability (`net`), the handle table, and the read and write paths all exist
already — this is the same five lines that `tcp_listen` needed. It is the
smallest outstanding gap in this directory and it blocks the most.

Worth noting what would *still* be missing afterwards: TLS, so it could fetch
`http://` and not `https://`, which is most of the web. And the safety property
the project is really testing — that concurrent writes to shared state are
refused — is already demonstrated by `par_map`, which the compiler refuses to
let capture a `ref` at all ([E-PAR-REF]).
