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
| Now | 285 |
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


## P4 — the region sweep, assessed

A proposal arrived to fix Grasm's performance by turning on
`REGION_SWEEP` in `crates/grasm_native/src/lib.rs:37`, replacing per-local
reference counting with a bump allocator. It is worth answering carefully,
because the diagnosis is partly right and the fix would not work.

**What the proposal gets right.** The flag is real and off. Allocation cost is
the biggest single factor in the worst benchmark: `tree` runs at 5.1x Rust and
carries the 0.45x geometric mean almost by itself — the other six sit at
1.6-2.3x. The runtime half of the sweep is written and tested: `ARENA`,
`grasm_region_push` and `grasm_region_pop` all exist.

**Why turning it on changes nothing today.** `region` is opt-in syntax, and
**no program in this repository uses it** — zero blocks across 584 `.gr` files,
including every benchmark. Enabling the sweep would alter the performance of
nothing currently written.

**Why the benchmark that matters cannot use it.** `[T-REGION]` says a region
may not write to an outside binding, may not `return` through the brace, and
yields a scalar or nothing. `tree.gr` is `fn build(...) -> Node` — it exists to
*return* an allocated tree. The 5.1x program is structurally ineligible for the
mechanism proposed to fix it.

**Why it is off is recorded, and it is not neglect.** The comment at the flag
says: a `ref` written through a call, and an effect handler written outside the
block, both carry a region-allocated value past the brace. With the sweep on,
those are **use-after-free**. Turning it on today trades a 2.5x slowdown for
memory corruption.

**What would actually help, in order.**

1. Close the two escape routes in the checker. That is the real work, and it is
   what the flag is waiting on.
2. Then enable the sweep — which makes `region` worth writing, and makes the
   tier's promise true.
3. Separately, and larger: the general win needs escape analysis that does not
   require opt-in syntax, so an allocation that provably does not escape its
   function is stack-allocated without anyone writing `region`. That is the
   change that would move `tree`, and it is a different project from the flag.

One more piece of evidence against "allocation costs drop to near-zero": the
runtime records that a `Cell<usize>` depth counter, added so an allocation
could ask "am I in a region" with a load instead of a `RefCell` borrow,
measured **zero** on a benchmark allocating 800,000 times. The bookkeeping
around allocation is not where the time goes.

## Tier 1 — to 200: done

Two hundred libraries, every one of which compiles, formats and lints clean,
declares its capabilities, has its types prefixed so nothing collides, and is
asserted against by a group demo that runs under all four engines.

Six names from the original list turned out to be packages that already
existed under another name, and writing them would have been the failure
`docs/packages-to-200.md` exists to prevent: `assertlib` and `fixture` are
`testing`, `flags` is `cli`, `spinner` is `progress`, `tablefmt` is
`texttable`, `plurals2` is `pluralize`, and `dirwalk` and `globwalk` are both
`fs`. `totp` was written before this was noticed and folded into the `hotp`
that already did the job. Their places went to `atomicfile`, `envcfg`,
`ulid`, `iban`, `phonetic`, `natsort` and `fixtureset`, each of which
something in the set actually needed.

Five of the two hundred are gate harnesses rather than libraries —
`casefolddiff`, `certdiff`, `cryptodiff`, `wasmdiff` and `zlibdiff` — and
they are counted because they are packages that exist and are built, not
because the number needed rounding up.

## Tier 2 — to 500

Three hundred more. Grouped by what unblocks each group, because two of the
four groups cannot be started at all.

### Buildable now — about 180

These need nothing that does not exist. Written in batches of five or six
with a group demo each, the same rhythm Tier 1 used, and checked against an
outside implementation wherever one exists.

| batch | packages |
|---|---|
| ~~Archives~~ | ~~`zip` `tar` `lz4` `snappy` `arfile`~~ — done |
| ~~Serialisation~~ | ~~`avro` `bson` `plist` `properties` `netstring`~~ — done |
| ~~SQL~~ | ~~`sqlbuild` `sqlparse` `sqlescape` `sqlitefile` `dbschema`~~ — done |
| ~~Web routing~~ | ~~`router` `middleware` `session` `csrf` `etag`~~ — done |
| ~~Web content~~ | ~~`htmlparse` `cssparse` `xpath` `staticfiles` `rangeheader`~~ — done |
| ~~Schemas~~ | ~~`jsonschema` `openapi` `graphqlparse` `protoschema` `avroschema`~~ — done |
| ~~Time~~ | ~~`calendar` `recurrence` `workdays` `iso8601` `humanize`~~ — done |
| ~~Geography~~ | ~~`geojson` `geohash` `haversine` `polyline` `wkt`~~ — done |
| ~~Linear algebra~~ | ~~`linalg` `fft` `polynomial` `quaternion` `bezier`~~ — done |
| ~~Statistics~~ | ~~`regression` `distributions` `hypothesis` `sampling` `outliers`~~ — done |
| ~~Game and graphics~~ | ~~`collision` `quadtree` `astar` `tilemap` `palette`~~ — done |
| ~~Networking~~ | ~~`whois` `tftp` `socks5` `gopher` `netdiscovery`~~ — done |
| ~~Observability~~ | ~~`otelspan` `prometheus` `logsample` `errorgroup` `slo`~~ — done |
| ~~Terminal UI~~ | ~~`tuibox` `keymap` `editline` `prompt` `autocomplete`~~ — done |
| ~~Cryptography~~ | ~~`argon2` `bcrypt` `rsa` `ed25519` `pemfile`~~ — done, plus `blake2b` |
| ~~Certificates~~ | ~~`csr` `pkcs7` `ocsp` `jwk` `certchain`~~ — done, plus `hostmatch`, `derwrite`, `pkcs8` and `crl` |
| Text analysis | `stemmer` `stopwords` `readability` `hyphenate` `keyphrase` |
| Documents | `rst` `bbcode` `wikitext` `csvdialect` `fixedwidth` |
| Build tooling | `changelog` `spdx` `codeowners` `patchfile` `coveragefmt` |
| Money and tax | `vatid` `taxrate` `invoice` `ledgerfmt` `fx` |

That is a hundred, in twenty batches. The second hundred is deliberately not
listed: the first will change what the second should be, and a list written
now would be a list written before anything was learnt.

### Needs P1 — a TLS client. About 120, and none of them can start.

Every hosted-service client: Stripe, AWS, Google Cloud, Azure, Discord,
Slack, GitHub, Kubernetes, Prometheus remote-write, OpenTelemetry export,
OAuth providers, SMTP and IMAP over TLS, and every managed database. All of
them are HTTPS-only and have been for years.

**Four of the five missing pieces are now built**, and each was checked
against certificates and signatures produced by an outside implementation:

- `rsa` — PKCS#1 v1.5 and PSS *verification*. No signing: a private key
  operation without constant-time arithmetic and blinding leaks the key, and
  verification is the half a certificate chain needs.
- `certchain` — path building from a leaf through intermediates to a trust
  store, validity windows across the whole path, basic constraints, path
  length and key usage. The signature check is a function the caller passes
  in, so this package contains no cryptography and the caller decides which
  algorithms it will accept.
- `hostmatch` — RFC 6125. The Common Name is taken and ignored; a wildcard
  covers one leftmost label and nothing else; an address is matched against
  address entries and never against names.
- `pemfile` — a chain file is several blocks, and a reader that takes the
  first verifies the leaf against nothing.

**What is left is the handshake over a socket.** That cannot be asserted
against without a server to talk to, which is why the four above were done
first: they are the parts a fixture can check exactly, and they are the parts
that were actually missing.

### Needs P2 — async I/O. About 40.

Kafka, RabbitMQ, MQTT, task queues, event loops, connection pools, WebSocket
servers at scale. A thread per connection works to a few hundred; past that
it needs `poll` or `epoll`/`kqueue`, which is a runtime addition rather than
a library.

### Needs P3 — a decision. About 90.

ONNX, TensorFlow, PyTorch, Qt, OpenGL, Vulkan, FFmpeg, USB, serial,
Bluetooth. Still an open question for the project owner, not a task: see P3
above.

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
