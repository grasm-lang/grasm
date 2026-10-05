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

## The linchpin the list does not name

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

It is also the hardest thing on any of these lists, because four engines have
to agree byte-for-byte and a detached task's interleaving is not deterministic.
The mechanism that makes it testable already exists: the runtime journals 34
nondeterministic operations and `replaytest` holds every one of them to it.

So the order is: detached tasks, then `bindgen`, then the OS primitives — in
that order, because the first is a language change that everything else is
written against, and because the second turns twenty months of wrapper work
into days.
