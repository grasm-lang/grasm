# proxy: an HTTP/1.1 reverse proxy subset

A client connects to the proxy, which forwards each request to an upstream
server and relays the response. Rewriting the request means parsing its head
and inserting exactly one `X-Forwarded-For: <client ip>` header, or appending
`, <client ip>` to the one the client sent. The benchmark measures that and the
byte relaying around it. It also tests whether each language can do this
without copying the body.

| piece | what it is |
|---|---|
| `upstream/` | Go `net/http` keep-alive server, shared by every run. Returns a fixed body (`-size`, default 4096). Echoes the `X-Forwarded-For` it received (`X-Echo-Xff`), how many such headers it got (`X-Echo-Xff-Count`) and the request body length (`X-Echo-Len`). |
| `loadgen/` | Go raw-socket client: `-c` keep-alive connections (256), `-n` requests (200,000). Checks every response for status 200, body length, `X-Forwarded-For` exactly as expected and exactly once, and the request body length. Reports requests/s, p50/p99/p999 latency, and the proxy's peak RSS from `ps -o rss=` every 100 ms. |
| `grasm/proxy.gr` | `net_workers` threads (32 by default), each running a `net_wait` readiness loop over the shared listener and its own clients. |
| `rust/` | tokio (1.53.1) with one task per client. |
| `go/` | Raw `net.Conn` with one goroutine per client. It does not use `httputil.ReverseProxy`. |

Request mix, by request index `i`:
- `i%10==0`: a POST with a 100-byte body.
- `i%7==0`: the client sends its own `X-Forwarded-For`, which the proxy must append to.
- Otherwise: a GET with no body.

All three proxies implement the same subset:
- Requests and responses are framed by `Content-Length` only. Chunked encoding is not supported in either direction.
- Each client gets one upstream keep-alive connection, opened when the client connects.
- One request is in flight per client. Pipelined requests are buffered and served in order.
- The client IP is the peer address of the accepted connection: `peer.ip()` in Rust, `RemoteAddr()` in Go and `tcp_peer` in Grasm. The `X-Forwarded-For` bytes are built once per client, when it connects.
- Nagle is off on every proxy's sockets. Grasm turns on TCP_NODELAY for every accepted and dialed stream.

## Running

```
./verify.sh          # 4 connections, 1,000 requests per proxy; fails unless 0 errors
./run.sh             # full size; prints name, req/s, p50/p99/p999 ms, peak RSS MB, errors
CONNS=64 REQS=20000 PROXIES="grasm go" ./run.sh
```

`build.sh` (which both scripts call) builds everything into `$BUILD_DIR`. The default is `$TMPDIR/proxybench-build`. It compiles the Grasm proxy with `$GRASM`, which defaults to `target/release/grasm` at the repository root. Rust builds offline into `$CARGO_TARGET_DIR`. The scripts use ports `$PORT_BASE` and `$PORT_BASE+1` (default 23300 and 23301).

**Parallelism.** Rust (tokio multi-thread) and Go use every core. The Grasm proxy runs `net_workers(n, f)` ([L-LIB-NET-WORKERS]): `n` OS threads, each running the readiness loop. The third argument sets `n` (`proxy 9301 9300 4`); the default is 32.
- Why 32 on an 8-core machine: a client stays on the worker that accepted it, so when the OS pauses a worker, every client it holds waits. Tokio moves runnable tasks to idle threads; Grasm workers share nothing but the listener and cannot. With the load generator and the upstream also on the machine, 8 workers of 32 clients each put p99 at 22–32 ms against Rust's 11–14 ms at the same requests/s; 32 workers of about 8 clients each put it at 14–18 ms, with requests/s unchanged (51–60k for all three, load 3–11, 2026-10-02). The remaining tail gap is this, not the per-request work.
- On macOS `net_wait` keeps one kqueue per thread with the wait set registered, and changes only what differs between calls, so a wait costs what is ready rather than what is open. With `poll` it cost 1.4x Rust's CPU per request in system time alone.
- All workers wait on the one listener. A new client wakes several of them, and they all call `tcp_accept`. One gets the connection. For the others, `tcp_accept` returns an error instead of waiting, because an accept right after `net_wait` reported the listener ready never waits. Each worker accepts one client per wake-up, so a burst of clients is spread across workers.
- A client and its upstream connection stay on the worker that accepted the client. Workers share nothing else: the checker allows the worker function only the `net` effect, and no `ref` or actor handle from outside.
- Handles are process-wide. The runtime's socket table is one locked table, and no lock is held during blocking I/O.
- `spawn` still does not carry `net`. A `go` task blocked in `tcp_accept` or `tcp_read` still blocks every other task. `net_workers` is the only way to put sockets on several threads.

## Where each proxy copies

- **Rust and Go:**
  - The request is parsed in place in the read buffer.
  - It is forwarded with one vectored write (`write_vectored` / `net.Buffers`) of three borrowed slices: the head up to the insertion point, the header bytes, and the rest including the body. The request is not copied.
  - Response chunks are written straight out of the read buffer.
  - The only copy is a response head that straddles two reads. The buffer is compacted after a request, which matters only with pipelining.
- **Grasm:**
  - The parsers take `bytes` as plain (lent) parameters and read it with `at`, so nothing is copied to parse.
  - Response chunks are relayed uncopied: the `bytes` value `tcp_read_bytes` returns is passed to `tcp_write_bytes` as is.
  - The request is copied about four times: two `slice` calls and two `concat` calls, body included. `bytes` has no view type, `slice` and `concat` allocate, and no call writes a sub-range or several buffers at once.
  - A request split across reads is joined with `concat`.

## Grasm gaps found while writing this

Fixed since the benchmark was written:

1. **Fixed: writing a list element through a struct field copied the whole list in native builds.** `s.xs[i] = v` cost O(len) on every write: 20k writes into a 1M-element field took about 4.8 s natively and 10 ms under `grasm run`. For a `[bytes]` field each write also touched every element's refcount. A nested write now moves the child list out of an unshared parent and back, so the same 20k writes take about 1 ms. `proxy.gr` used to route every write through `put_i` / `put_b` (`inout` list parameters). It now writes `st.kind[h] = v` directly.
2. **Fixed: native panic "index on non-list".** It happened when an argument read the same field another argument passed as `inout`, for example `put(s.xs, 1, s.xs[1] + 1)`. The place is now copied when a later argument names the same root, and the native build prints `[1, 3, 3]` like `grasm run`.
3. **Fixed: no peer address.** `tcp_peer(h)` returns the address and port at the far end of an accepted or dialed stream. The proxy puts the real client IP in `X-Forwarded-For`, as Rust and Go do. It used to put the constant `127.0.0.1` there.
4. **Fixed: loopback only.** `tcp_listen_on(host, port)` binds any address, for example `"0.0.0.0"`. `tcp_listen` still binds `127.0.0.1`, and the benchmark keeps using it.
5. **Partly fixed: socket options.** TCP_NODELAY is now on for every accepted and dialed stream. The listen backlog still cannot be set, and no other option can be set.
6. **Fixed: `bytes.at` was a runtime call per byte.** Natively it is now an inline bounds check and one load. An out-of-range index still takes the call so that it panics with the same message.

Still open:

7. **Blocking writes.** `net_wait` reports readability only, and `tcp_write_bytes` blocks until everything is written. One slow reader stalls every client on the same worker.
8. **`bytes` API gaps:**
   - No view or sub-range type.
   - No `index_of`.
   - No writev and no "write `b[from..to]`".

   So the request is still copied about four times (see above).
9. **Every read pins a fresh 64 KiB buffer.** `tcp_read_bytes` allocates 64 KiB per call and truncates without shrinking, so each returned value keeps the allocation. Holding 1,000 one-byte reads costs about 18 MB RSS.
10. **Wait-set cost grows with the set.**
    - `net_wait` rebuilds a `poll(2)` set on every call: O(n) per call, with no kqueue or epoll.
    - The handle table finds a free slot with a linear scan.
11. **Fixed: no parallelism for sockets.** `net_workers(n, f)` runs `f` on `n` threads that share the socket table (see "Parallelism" above). A worker cannot hand a connection to another worker, so a busy worker's clients cannot move to an idle one.
