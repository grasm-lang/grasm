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
| Now | 513 |
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

### ~~P2 — Async I/O or a socket poll.~~ **The poll is built.** About 40
unblocked.

This was written as "a thread per connection works to a few hundred; past that
it needs `poll` or `epoll`/`kqueue`, which is a runtime addition, not a
library". The runtime addition exists and has since before 1.0: `net_wait(hs,
ms)` is `poll(2)` behind [L-LIB-NET], `packages/http_server` walks its answer
in `serve_many`, and the entry stayed on the blocked list after it stopped
being true.

Measured on 2026-09-10, by opening every connection *before* sending anything
so the server is holding all of them at once rather than serving them in turn:

| connections held open | answered | wall clock |
|---:|---:|---:|
| 50 | 50 | 4 ms |
| 200 | 200 | 17 ms |
| 1000 | 1000 | 100 ms |
| 2000 | 2000 | 266 ms |
| 3000 | 3000 | 378 ms |

One thread, one process, every request answered. It stops where the file
descriptor limit stops it, not where the design does.

What is honestly still missing is `epoll`/`kqueue`. `poll(2)` walks its whole
set on every call, so the cost per pass grows with the number of idle
connections — fine at the thousands above, the wrong shape at a hundred
thousand. A broker at that scale wants an edge-triggered interface. Nothing
below needs one.

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
| ~~Text analysis~~ | ~~`stemmer` `stopwords` `readability` `hyphenate` `keyphrase`~~ — done |
| ~~Documents~~ | ~~`rst` `bbcode` `wikitext` `csvdialect` `fixedwidth`~~ — done |
| ~~Build tooling~~ | ~~`changelog` `spdx` `codeowners` `patchfile` `coveragefmt`~~ — done |
| ~~Money and tax~~ | ~~`vatid` `taxrate` `invoice` `ledgerfmt` `fx`~~ — done |

That is a hundred, in twenty batches.

### The second hundred — about 100, and what the first taught

The first hundred is done, so this list can be written from evidence rather
than from a guess. Four things changed how it was chosen.

**Check against somebody else's implementation, always.** It found something
in every batch. Usually the expectation was wrong and the code right — a
colour-blindness matrix applied in the wrong space, a Bezier arc length out in
the ninth digit, a `both_ways` VAT total that agreed when it should not — and
once it was the code: `vatid` had six real defects that 3,267 generated
numbers found and no amount of reading would have. So every batch below names
the implementation it will be checked against, and a package with no reference
has to justify itself.

**The demo's running time is a design constraint.** `crypto4-demo` takes 18
seconds in the interpreter because Ed25519 and bcrypt are meant to be slow,
and it runs four times in the gates. That is acceptable once. It is not
acceptable in every batch, so the expensive ones are spread out below rather
than grouped.

**Packages that pair up are worth more than packages that stand alone.**
`patchfile.changed_lines` feeding `coveragefmt.changed_line_rate` says
something neither says by itself. Batches below are grouped so that at least
one pair meets inside the group demo.

**Data is fine; unsourced data is not.** `hyphenate` carries 39 KB of TeX
patterns and `spdx` 13 KB of identifiers, and both are better for it. Each
says where the data came from and under what licence, which is the condition.

| batch | packages | checked against |
|---|---|---|
| ~~Spatial~~ | ~~`convexhull` `kdtree` `rtree` `simplify` `triangulate`~~ — done | scipy, shapely, earcut |
| ~~Graph algorithms II~~ | ~~`maxflow` `matching` `pagerank` `centrality` `spanning`~~ — done | networkx, scipy |
| ~~Numerical methods~~ | ~~`rootfind` `quadrature` `ode` `optimise` `autodiff`~~ — done | scipy |
| ~~Fonts~~ | ~~`sfnt` `cmap` `glyf` `woff` `fontmetrics`~~ — done | fontTools |
| ~~Barcodes~~ | ~~`code128` `ean` `itf` `codabar` `code39`~~ — done | python-barcode |
| ~~Audio metadata~~ | ~~`id3` `flacmeta` `midi` `wavmeta` `loudness`~~ — done | mutagen, mido, pyloudnorm |
| ~~Images II~~ | ~~`gif` `ico` `jpegmeta` `resample`~~ and `bmpread`, because `bmp` already writes — done | Pillow |
| ~~Text encodings~~ | ~~`codepage` `shiftjis` `gb18030` `big5` `euckr`~~ — done | Python codecs |
| ~~Compression II~~ | ~~`bzip2` `lzw` `huffman` `rangecoder` `deltacodec`~~ — done | Python bz2, `/usr/bin/compress`, Pillow |
| ~~Protocols II~~ | ~~`mqtt` `coap` `stomp` `amqpframe` `wsextension`~~ — done | paho, aiocoap, stomp.py, pika, websockets |
| ~~Database internals~~ | ~~`bplustree` `heapfile` `queryplan` `costmodel` `bitmapindex`~~ — done | SQLite's own plans |
| ~~Compiler tooling~~ | ~~`lexgen` `parsegen` `firstfollow` `ssaform` `regalloc`~~ — done | Lark, networkx, Python re |
| ~~Scientific units~~ | ~~`siunits` `constants` `uncertainty` `dimension` `unitparse`~~ — done | pint, CODATA via scipy, uncertainties |
| ~~Time zones~~ | ~~`tzfile` `tzrule` `sunrise` `solar` `leapsecond`~~ — done | zoneinfo, astral, IERS |
| ~~Statistics III~~ | ~~`bootstrap` `anova` `survival` `timeseries` `changepoint`~~ | scipy, statsmodels, lifelines, pingouin, ruptures |
| ~~Machine learning basics~~ | ~~`kmeans` `naivebayes` `decisiontree` `knn` `mlmetrics`~~ | scikit-learn |
| ~~Security III~~ | ~~`passwordpolicy` `entropy` `secretscan` `keyderive` `saltedhash`~~ | zxcvbn, gitleaks' own rules, passlib |
| ~~Web III~~ | ~~`formdata` `csp` `hsts` `preflight` `webfinger`~~ | RFC 6797's own ABNF, Python's `email`, `content-security-policy`, Flask-Cors, urllib |
| ~~Serialisation IV~~ | ~~`flatbuffers` `capnp` `ubjson`~~ and `ion` and `thriftcompact`, because Smile's and AMF's only Python implementations are abandoned at Python 2 and there is nothing left to check against | the reference encoders |
| ~~Geospatial II~~ | ~~`mgrs` `utm` `gpx` `kml` `geodesic`~~ | pyproj, geographiclib, the Python `mgrs` package, gpxpy |
| ~~HTTP/2~~ | ~~`http2frame` `http2client` `grpcwire`~~ — and `hpack`'s Huffman coding, which was the gap that blocked all three | hyperframe, Python `hpack`, a live `h2` server |
| ~~APIs I~~ | ~~`urltemplate` `jsonpointer` `jsonpatch` `jsonmergepatch` `stripewire` `s3wire`~~ | the RFC 6570 and JSON Patch test suites, RFC 7386's own appendix, Stripe's encoder, botocore |
| ~~Services~~ | ~~`k8squantity` `discordwire` `githubwire` `slackwire` `gitignore`~~ | the Kubernetes client, discord.py, `git check-ignore` itself |
| ~~Unicode~~ | ~~`unicodenorm` `graphemes` `wordbreak` `textwidth`~~ | the Consortium's own NormalizationTest, GraphemeBreakTest and WordBreakTest, and wcwidth |
| ~~Plumbing~~ | ~~`shellwords` `datefmt` `siphash` `zonefile` `cidr`~~ | Python's `shlex`, `datetime` and `ipaddress`, the SipHash paper's vectors, dnspython |
| ~~Codes~~ | ~~`bigrational` `bech32` `checkdigits` `mnemonic` `base45`~~ | Python's `fractions`, BIP-350's and BIP-39's own vectors, python-stdnum, RFC 9285 |
| ~~Unicode II~~ | ~~`sentencebreak` `casemap` `unicodescript` `numericvalue`~~ | the Consortium's SentenceBreakTest, Python's `str.upper`, the `regex` module's script property, `unicodedata` |
| ~~Layout~~ | ~~`structuredfields` `linebreak` `wordwrap`~~ | the HTTP working group's structured-field suite, UAX #14's LineBreakTest |
| ~~Personal data~~ | ~~`contentline` `icalendar` `vcard`~~ | the Python `icalendar` library's line reader, `vobject` |
| ~~Media and citation~~ | ~~`m3u8` `subtitles` `bibtex`~~ | the `m3u8` library, `srt` and `webvtt-py`, `bibtexparser` |
| ~~Configuration~~ | ~~`gitconfig` `sshconfig` `editorconfig`~~ | `git config --list` itself, paramiko, the reference EditorConfig library |
| ~~Version schemes~~ | ~~`pep440` `pep508` `semverrange` `debversion`~~ | Python's `packaging`, node's `semver`, `python-debian` |
| ~~Operations files~~ | ~~`netrc` `dockerfile` `nginxconf` `knownhosts`~~ | Python's `netrc`, `dockerfile-parse`, nginx-inc's `crossplane`, `ssh-keygen` itself |
| ~~Network II~~ | ~~`ipv6` `packets` `pcapfile`~~ | Python's `ipaddress`, `dpkt` |
| ~~Package identity~~ | ~~`rpmversion` `gemversion` `wheelname` `purl` `cpename`~~ | rpm's own `rpmvercmp` suite, RubyGems' `Gem::Version`, `packaging`, the purl-spec test suite, the `cpe` library |
| ~~Executables~~ | ~~`elfread` `peread` `machoread` `magicbytes`~~ | pyelftools, `pefile`, `otool`, `file(1)` |
| ~~Packages and images~~ | ~~`cpio` `rpmfile` `iso9660` `checksumfile`~~ | `cpio(1)` both ways, the `rpmfile` library over real Fedora packages, `pycdlib`, `shasum` |
| ~~Mail~~ | ~~`mimeword` `emailaddr` `mboxfile` `ldif` `spf` `dkimsig`~~ | Python's `email` and `mailbox`, `ldif3`, `pyspf`, `dkimpy` |

That is a hundred, in twenty batches, and the same rhythm as the first: five
or six packages to a group demo, every claim an assertion, every batch checked
against the tool that owns its format before it lands. **All twenty are
done**, and the batches after them are the first that P1 unblocked.

**The 500 mark is passed: 513.** That figure counts directories under
`packages/` that are not group demos, which is what "467" counted before it,
and 31 of them are gate harnesses rather than libraries — the small programs
the differential gates drive, listed in `scripts/`. Counting them is the same
choice the first two hundred made, and it is written here so the number can
be checked rather than believed.

The four are worth reading as a set, because each was checked against a
suite its own standard publishes rather than against expectations written
here: 270 URI templates, 123 JSON patches, 766 grapheme boundaries, 1944
word boundaries and 400,680 normalisations. Two of those found defects —
`json.equal` compared object members by position, which made a JSON Patch
`test` fail against any server that reordered a response, and a `.gitignore`
negation was allowed to reach into a subtree it never named.

### ~~Needs P1 — a TLS client.~~ **P1 is built.** About 120 unblocked.

Every hosted-service client: Stripe, AWS, Google Cloud, Azure, Discord,
Slack, GitHub, Kubernetes, Prometheus remote-write, OpenTelemetry export,
OAuth providers, SMTP and IMAP over TLS, and every managed database. All of
them are HTTPS-only and have been for years, and none of them could be
started until now.

`https_client` speaks HTTP/1.1 over it, `awssig` signs what AWS wants signed
and `webhooksig` checks what arrives back, so the first layer above TLS is
built too — and above that `cloudevents`, `problemjson` and `httpcache`, the
three shapes a hosted API answers in.

`http2frame`, `http2client` and `grpcwire` speak the version of HTTP the
hosted services above actually run, and the RPC protocol built on it. What
made this possible was finishing `hpack`: its Huffman coding was a gap, and
a gap in HPACK is total, because a server compresses whichever way it likes
and one that chooses Huffman is one this client could not read a header
from. The `h2diff` gate replays a recorded conversation from Python's `h2`
library, including a response larger than a default connection window, so a
client that never refills flow control fails rather than hangs.

The service clients themselves have started: `stripewire` writes the form
encoding Stripe's API takes and reads the errors it answers with,
`s3wire` decides which of the two ways to address a bucket a request has to
use and reads a listing back, `discordwire` does the two pieces of
arithmetic a Discord client cannot avoid, `githubwire` handles the two rate
limits and the pagination that is not in the body, `slackwire` refuses a
message before Slack does, and `k8squantity` reads a Kubernetes resource
limit exactly rather than through a float. `urltemplate`, `jsonpointer`,
`jsonpatch` and `jsonmergepatch` are the four things every one of those
APIs uses to name and to edit a resource, and each is checked against the
test suite its own specification publishes.

`tlsclient` completes a TLS 1.3 handshake over a socket and refuses to
return a connection it has not verified. `tlshandshake` builds and reads the
messages; `tlsverify` does the three checks — chain, name, signature over
the transcript — in that order. The `tlsdiff` gate runs five handshakes
against OpenSSL's own server on every gate run: one RSA, one ECDSA, and
three that must be refused.

The five pieces it is assembled from, each checked against certificates and
signatures produced by an outside implementation:

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

**The handshake over a socket is what was left, and it is done.** It could
not be asserted against without a server to talk to, so it is checked two
ways: the message layer and the key schedule against RFC 8448's published
byte-for-byte trace, which needs no network at all, and the whole client
against `openssl s_server` in a gate. The second found two defects the first
could not — a digest named `"sha256"` where the `rsa` package spells it
`"SHA256"`, which is a silent verification failure, and an ECDSA signature
handed to the curve code as bytes where it wanted limbs.

### ~~Needs P2 — async I/O.~~ **Unblocked, and the first one is written.**

Kafka, RabbitMQ, MQTT, task queues, event loops, connection pools, WebSocket
servers at scale. These wanted a readiness primitive and there is one: see the
measurement under P2 above, where one thread held three thousand connections
open and answered all of them. They are ordinary package work now, not work
behind a prerequisite.

`wsserver` is the first of them and the proof of it: a WebSocket server on one
thread, checked by `scripts/wsserver_diff.sh` with forty clients in front of
it at once, and during development by Python's `websockets` package at 250
clients and 1,750 messages with every round trip agreeing.

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

1. ~~**Finish Tier 1 to 200.**~~ Done.
2. ~~**Build P1, the TLS client.**~~ Done: chain validation, a trust store,
   RSA verification, hostname checks.
3. ~~**Tier 2, the 180 that need nothing.**~~ Done.
4. ~~**Tier 2, the 120 behind TLS.**~~ Done — the count passed 500 on
   2026-09-10.
5. ~~**Build P2 if the broker and server packages are wanted.**~~ Already
   built, and the list did not notice: `net_wait` is `poll(2)` and one thread
   holds three thousand connections. The 40 behind it are ordinary work.
6. **Raise P3 as a question** rather than answering it inside a commit.
   Raised, and open: about 90 packages wait on the answer.

## What will not be claimed

A package that cannot connect will not be shipped as though it can. If the
TLS client is not finished, there will be no `stripe` package that speaks the
Stripe wire format to a socket nobody can open — that is a stub with a
convincing name, and the rule against those is the reason this document
exists.

## Known incompleteness in the compression packages

`deflate` writes fixed-Huffman blocks and stored blocks, and picks whichever
is smaller. It does not write dynamic-Huffman blocks, which describe their
own code tables and are where `zlib -9` spends its time; on the gate's own
corpus that is 17.8 per cent of the input against `zlib`'s 13.8. Adding them
needs the code-length alphabet, a Huffman code for that alphabet, and the
run-length encoding of the lengths — three more pieces, each with its own
edge cases, for roughly another quarter off text.

`xzfile` reads and does not write. The LZMA encoder is a much larger job than
the decoder — the decoder follows a specification, the encoder is a search
problem — and nothing in the tree needs to produce `.xz`.

Both are deliberate stopping points rather than oversights, written down here
so nobody has to read the source to find out.
