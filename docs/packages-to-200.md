# Getting the package set to 200

A working plan, and a tracker. Written down because "add 112 libraries" done
without one produces 112 stubs, and a stub is worse than a gap: a gap is
honest and a stub is a promise nobody kept.

## The rules each package has to pass

1. **It is something a real program needs.** If the answer to "who calls this"
   is "nobody, it rounds the number up", it does not get written.
2. **It compiles, is formatted, and lints clean.** `grasm check`, `grasm fmt`,
   `grasm lint` — all three, no findings.
3. **Its types are prefixed.** Types share one namespace program-wide, so a
   package declaring `Node` or `Entry` blocks every consumer of the other one
   that does. `fuzz_modules` checks the whole set and must stay at 0 holes.
4. **It is exercised by a demo that asserts, not prints.** Every claim in a
   demo is a `say(f, "...", condition)`, and the program says how many failed.
5. **Demos are grouped, about six packages to one.** Every demo runs under four
   engines in the agreement gates, so a demo per package would add roughly 450
   engine-runs. Grouping keeps the coverage, cuts the gate time, and exercises
   the multi-module bundler as a side effect.
6. **Where a standard exists, it is checked against the standard.** RFC
   vectors, not vectors invented here. Four fabricated test vectors have
   already been caught in this project; the code was right every time and the
   expectation was wrong.

## Where it stands

| | |
|---|---|
| Started at | 78 |
| Now | 451 |
| Target | 200 |
| Remaining | 0 |

## Batches

Ticked when the package compiles, formats, lints clean, and its group demo
asserts against it.

### Done

- [x] **Structures I** — `heap` `deque` `bitset` `unionfind` `semver`
- [x] **Text I** — `glob` `textwrap` `levenshtein` `base32` `ipaddr`
- [x] **Structures II** — `trie` `graphs` `lru`
- [x] **Structures III** — `roaring` `skiplist` `cuckoo` `btree` `tdigest`
  `ringlog`
- [x] **Numeric II** — `bigint` `fixedpoint` `complexnum` `interpolate`
  `units`
- [x] **Text III** — `casefold` `tokenizer` `wrapansi` `edit` `similarity`
- [x] **Crypto III** — `sha512` `sha3` `scrypt` `nonce`, and `totp` folded
  into the `hotp` that already existed
- [x] **Network wire formats** — `ntp` `syslog` `robots` `sitemap`
  `mimetype`
- [x] **System files** — `atomicfile` `pidfile` `lockfile` `shutdown`
  `envcfg`
- [x] **Formats II** — `msgpack` `cbor` `yamlish` `markdown` `template`
- [x] **Media II** — `png` `wav` `qrcode` `perlin` `dither` `asciiart`
- [x] **Config II** — `migrate` `locale` `i18n` `currency` `iban`
- [x] **Tooling III** — `benchtime` `treeview` `ulid` `phonetic` `natsort`
  `fixtureset`

**200 of 200.** Six names in the plan below turned out to be packages that
already existed under another name; `docs/ecosystem-roadmap.md` says which,
and what took their places.

### Planned, and what became of it

**Encoding** — `base58` `base85` `cbor` `msgpack` `bencode` `hexdump`
`punycode` `utf16`

**Numeric** — `bigint` `rational` `complexnum` `stats` `fixedpoint`
`geometry` `combinatorics` `primes` `adler` `numfmt`

**Crypto** — `sha512` `sha3` `pbkdf2` `hmac` `ed25519` `constanttime` `totp`
`nonce`

**Text II** — `markdown` `yamlish` `ini` `jsonpath` `slug` `ansi` `texttable`
`casing` `pluralize` `template` `difflines` `wordfreq`

**Network** — `macaddr` `ntp` `syslog` `statsd` `smtp` `sse` `useragent`
`robots` `sitemap` `mimetype` `contentneg` `retryafter`

**Analytics** — `hyperloglog` `countmin` `tdigest` `cuckoo` `roaring`
`reservoir` `histogram` `topk`

**System** — `tempname` `lockfile` `cron` `ratelimit` `semaphore` `backoff`
`healthcheck` `shutdown` `pidfile` `dirwalk`

**Tooling** — `assertlib` `fixture` `benchtime` `logfmt` `flags` `spinner`
`progress` `diffview` `tablefmt` `envcfg`

**Media** — `png` `bmp` `wav` `qrcode` `colorspace` `easing` `perlin`
`dither` `asciiart`

**Config** — `schema` `migrate` `i18n` `locale` `currency` `money` `tzoffset`
`duration`

## What this deliberately does not include

A web framework, an ORM, a game engine. Each is a project rather than a
package, and shipping a thin one to raise a count is the failure mode this
document exists to prevent.
