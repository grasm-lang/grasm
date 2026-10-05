# The hundred-package list, audited against what exists

A curated list of "the top 100 packages missing in Grasm to reach parity with
crates.io and Go" arrived on 2026-10-05. It is a good list and two of its
premises are wrong, so this is the audit rather than the plan that followed
from it.

**There are 651 packages.** `docs/roadmap.md` §3 says thirty-four, which was
true when it was written and has been stale for a long time. Against the 651:

| | count | what it means |
|---|---|---|
| already there | 12 | `router`, `jwt`, `cors`, `argon2`, `bcrypt`, `fft`, `glob`, `openapi`, `multipart`, `prometheus`, `qrcode`, `autodiff` |
| the hard half is there | 42 | the protocol, format or algorithm is written; what is missing is a binding to the real system or an ergonomic layer |
| genuinely absent | 46 | |

**TLS is not a blocker.** `docs/packages-plan.md` lists TLS first under
"Blocked, and on what", and says there will not be a hand-written one. There
is: `tls13` is the RFC 8446 key schedule and record layer in Grasm, with
`tlshandshake`, `tlsverify`, `tlsclient`, `x509`, `certchain` and an
`https_client` above it. So HTTPS, `wss://` and OAuth2 against a real provider
are not waiting on anything. That doc needs the same correction as the
roadmap, and the measurement to go with it: whether this TLS is fast and
whether it has been fuzzed against a real server are open questions, and are
different questions from whether it exists.

## What the 46 absences actually need

Grouping them by the one thing that would unlock them, rather than by domain:

**A C binding generator — 20 of the 46.** `duckdb`, `onnx`, `ggml`, `ffmpeg`,
`cpal`, `av1`, `libusb`, `hid`, `serialport`, `vulkan`, `metal`, `wgpu`,
`winit`, `sdl`, `raylib`, `egui`, `slint`, `clipboard`, `filedialog`,
`keyring`. Every one of these is a wrapper around a C library that already
exists and that nobody should rewrite. Grasm has `extern fn` with `unsafe` in
the effect row already ([T-EXTERN], and the conformance programs for it), so
this is a *code generator* reading a `.h` file, not a language feature. The
list calls it `grasm-bindgen` and ranks it #1, and on this evidence that is
right.

**Operating-system primitives — 10.** `mmap`, `shm`, `ipc`, `io-uring`,
`signals`, `sysinfo`, `dlopen`, `notify`, `which`, `pty`. Each is a small set
of syscalls in the runtime plus a package over it. They are independent of one
another and each is roughly a day. `signals` and `notify` are the two that
most other things want.

**Already possible, simply unwritten — 12.** `quic`, `http3`, `svg`, `nats`,
`cassandra`, `dynamodb`, `webauthn`, `wireguard`, `lsp-server`, `arrow`,
`parquet`, `mp3`. Nothing blocks these; they are work.

**`cxx` and `glyph-atlas`** need, respectively, C++ name mangling and a GPU,
and should wait.

## The linchpin that turned out not to be one

Its own "five linchpins" section picks `bindgen`, `sqlite`+`pool`,
`web`+`router`, `pty`+`notify`, and `wgpu`+`winit`. The first is right. The
others mostly have their hard halves already — `sqlitefile`, `connpool`,
`http_server`, `middleware`, `router`, `termios`.

What they are waiting on is not on the list at all: **a task that outlives its
block.** `parallel` and `spawn` join before the next statement, by design and
for determinism, and nothing detaches. So a connection pool cannot keep a
socket warm between requests, a web framework cannot have a background worker,
and there is no async runtime for a driver to be async *on*. That single
limitation sits under `pool`, `web`, `tower`, `grpc`, `quic`, `http3`, `kafka`,
`nats`, `etcd`, and every database driver the list calls async — about 25 of
the 100, counting the partial ones.

### Except it does not. Measured, 2026-10-05.

Everything above this line was written before trying it, and the paragraph
before it is wrong. [E-SCHEDULE] is a *cooperative deterministic scheduler*
already: a `go` task runs on its own fiber until a channel operation has to
wait, then the next runs, in a fixed order. Tasks are long-lived fibers, not
statements. So what the twenty-five packages are said to need was tested:

- **A background task holding state across many requests.** Works. A `go`
  task looping on a channel kept a socket count, served six requests from
  three "generations" and reaped between them, deterministically, on the
  interpreter and natively with identical output.
- **A library providing the loop and the consumer providing the task.** Works,
  including yielding across the function call — `go { pool_loop(work, done) }`
  suspends inside `pool_loop` at its `chan_recv` and resumes there.

One thing genuinely does not work, and it is narrower than "detached tasks":

- **A function cannot return while a task it started is still running.**
  `concurrent` does not exit until its tasks end, so a library cannot hand
  back an opaque handle with a task running behind it. Tried, and it is a
  clean deadlock panic naming the waiting tasks rather than a hang.

That is the structured-concurrency bargain every language making this choice
has taken — Trio's nurseries, Rust's scoped threads, Kotlin's
`coroutineScope` — and it is a constraint on an API's *shape*, not a missing
capability. A pool is written as a loop its consumer runs, not as a
constructor hiding a thread. The cost is that the task is visible in the
consumer's code; what it buys is that no task leaks and output is identical on
every engine and every run.

**So the ecosystem is not blocked on a language feature.** The 25 packages
need writing in the loop-and-channel shape. The order is `bindgen` first,
because it is worth twenty packages on its own, then the OS primitives, then
the unwritten pure-Grasm ones.

Detaching is still worth wanting for ergonomics, and QUIC's retransmission
timers are the one case where a yield-at-a-wait schedule is a real
approximation rather than a different spelling. Neither is a reason to put it
before `bindgen`.
