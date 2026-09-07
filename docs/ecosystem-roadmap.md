# The ecosystem roadmap: 200, then 500, then 800

A master plan, and — more usefully — a list of what blocks what. The order
below is not a preference. It is a dependency graph: several things in the
500 and 800 tiers **cannot be built at all** until three prerequisites exist,
and building around them produces packages that compile and cannot reach
anything.

Written down because a 600-package list with the blockers left implicit is a
plan that discovers them 300 packages in.

## Where it stands

| | |
|---|---|
| Started | 78 |
| Now | 136 |
| Tier 1 target | 200 |
| Tier 2 target | 500 |
| Tier 3 target | 800 |

Every package must pass the rules in `docs/packages-to-200.md`: it is
something a real program needs, it compiles, formats and lints clean, its
types are prefixed, a group demo asserts against it, and where a standard
exists it is checked against the standard rather than against itself.

## The three prerequisites

Nothing in this section is a package. Each is a piece of the language or the
runtime that a whole category of the plan depends on.

### P1 — A working TLS client. **Blocks about 120 packages.**

`packages/tls13` has the RFC 8448 key schedule and record layer.
`packages/x509` parses DER and verifies one ECDSA-P256 signature against one
key. Neither is a client. What is missing:

- the handshake driven over a real socket, start to finish
- **certificate chain building and a trust store** — today `x509` checks one
  signature against one key you supply, which is not "is this certificate
  trustworthy"
- RSA signature verification, because a large share of the public web still
  chains to RSA roots, and `p256` is ECDSA only
- hostname verification against SAN entries
- expiry and revocation checks

Until this exists, **every package below that talks to a hosted service is
unbuildable**: Stripe, AWS, Discord, GitHub, Kubernetes, Slack, Twilio,
SendGrid, every cloud SDK, every OAuth provider. They are all HTTPS-only and
have been for years. A package that speaks their wire format over plaintext is
a package that cannot connect.

This is the single highest-value item in the whole roadmap and it is the next
thing to build.

### P2 — Async I/O or a socket poll. **Blocks about 40 packages.**

Grasm has OS threads and structured concurrency and no async. That is enough
for a request/response client and not enough for:

- message brokers holding thousands of idle connections (Kafka, RabbitMQ, MQTT)
- an event loop, a task queue, a scheduler with backpressure
- a server handling more sockets than it has threads

A thread per connection works to a few hundred. Past that it needs `poll` or
`epoll`/`kqueue`, which is a runtime addition, not a library.

### P3 — A decision about C bindings. **Blocks about 90 packages, and it is
not a technical question.**

ONNX, TensorFlow, PyTorch, Qt, OpenGL, Vulkan, FFmpeg, libusb and every
Bluetooth stack are C or C++ libraries. `extern fn` reaches C with `i64`,
`f64`, and `str`/`bytes` as pointers — enough to bind a C API through a shim.

But a shim is C, and this project's stated principle is that everything is
written in Grasm. Binding these means either:

1. **writing C shims** — the bindings work, and the claim "no C" stops being
   true; or
2. **implementing the format natively** — a PNG encoder, a WAV decoder and a
   GeoJSON parser are all reasonable Grasm packages, while a Vulkan driver is
   not; or
3. **not shipping them**, and saying so.

Option 2 covers more of the list than it first appears. Option 1 is a decision
for the project owner, not one to be made silently inside a package. **This is
an open question, not a task.**

## Tier 1 — to 200 (64 remaining)

All buildable now. No prerequisite blocks any of it.

**Data formats** — `yamlish` `jsonpath` `msgpack` `cbor` `punycode`
`markdown` `difflines` `template`

**Structures** — `btree` `skiplist` `ringlog` `topk` `roaring` `cuckoo`
`tdigest`

**Numeric** — `bigint` `fixedpoint` `complexnum` `interpolate` `units`

**Text** — `casefold` `tokenizer` `wrapansi` `edit` `similarity`

**Crypto** — `sha512` `sha3` `ed25519` `nonce` `bcryptish`

**Network wire formats** — `ntp` `syslog` `sse` `robots` `sitemap`
`mimetype` `multipart` `cookiejar` `linkheader`

**System** — `tempname` `lockfile` `semaphore` `backoff` `healthcheck`
`shutdown` `pidfile` `dirwalk` `globwalk`

**Tooling** — `assertlib` `fixture` `benchtime` `flags` `envcfg` `tablefmt`
`spinner` `treeview`

**Media** — `png` `wav` `perlin` `dither` `asciiart` `qrcode`

**Config / i18n** — `migrate` `i18n` `locale` `currency` `plurals2`

## Tier 2 — to 500

Grouped by what unblocks each group.

**Needs nothing (about 180 packages).** Wire formats, codecs and pure logic:
Avro, Parquet, protobuf codegen, ORM query building, SQL construction and
escaping, a template engine, a router, middleware, session handling, CSV/Excel
readers, ZIP and tar, calendar maths, GeoJSON, collision detection, decimal
maths, and the CLI UI packages.

**Needs P1 — TLS (about 120 packages).** Every hosted-service client:
Stripe, AWS S3/DynamoDB/SQS, Google Cloud, Azure, Discord, Slack, GitHub,
Kubernetes, Prometheus remote-write, OpenTelemetry export, OAuth and OpenID
providers, SMTP/IMAP over TLS, MongoDB Atlas, and every managed database.

**Needs P2 — async (about 40 packages).** Kafka, RabbitMQ, MQTT, task queues,
event loops, connection pools, WebSocket servers at scale.

**Needs P3 — a decision (about 90 packages).** Native runtimes and GUI:
ONNX/TensorFlow/PyTorch bindings, Qt, OpenGL, Vulkan, FFmpeg, USB, serial,
Bluetooth.

## Tier 3 — to 800

Everything in Tier 3 sits behind a Tier 2 prerequisite, so it is listed as
direction rather than as a queue: data science and DataFrames, full cloud SDKs,
observability, desktop GUI, graphics, advanced web security, hardware access,
and the industry frameworks — FinTech, GIS, game development.

**One honest note on the AI tier.** "Machine learning wrappers" is a binding to
a C++ runtime, which is P3. A *native* tensor library — matrix maths, autograd,
a small inference engine for a known format — is buildable in Grasm and would
be genuinely novel. Grasm has one integer type and no SIMD, so it would be
correct and slow. Correct and slow is worth shipping; a wrapper that cannot be
built is not.

## The order this will be done in

1. **Finish Tier 1 to 200.** All buildable, no blockers, in flight now.
2. **Build P1, the TLS client.** Chain validation, a trust store, RSA
   verification, hostname checks. This is the one that unlocks the most.
3. **Tier 2, the 180 that need nothing.**
4. **Tier 2, the 120 behind TLS.**
5. **Build P2 if the broker and server packages are wanted.**
6. **Raise P3 as a question** rather than answering it inside a commit.

## What will not be claimed

A package that cannot connect will not be shipped as though it can. If the
TLS client is not finished, there will be no `stripe` package that speaks the
Stripe wire format to a socket nobody can open — that is a stub with a
convincing name, and the rule against those is the reason this document
exists.
