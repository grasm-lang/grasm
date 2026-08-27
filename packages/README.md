# packages

The libraries a first release needs, as packages the compiler holds to a
capability ceiling.

Each directory is a package with a `grasm.toml`. A `*-demo` beside a library is
an ordinary program that imports it, so the library is exercised by the same
gates as everything else: three engines, one answer.

| package | capabilities | what it is |
|---------|--------------|------------|
| `http` | *none* | Requests, responses, routing, escaping. Text in, text out. |
| `http_server` | `io`, `net` | The accept loop, and `net_wait` for many conversations on one thread. Depends on `http`. |
| `http_client` | `net` | Dials, sends what `http` composed, reads until the declared frame has arrived. Depends on `http`. |
| `json` | *none* | Parse, render, reach into a document, build one. |
| `bytes_codec` | *none* | Hex, base64, url-safe base64, constant-time compare. |
| `time` | *none* | Civil calendar, ISO-8601, durations. The clock is a parameter. |
| `random` | *none* | Seeded xoshiro256\*\*, uniform `below(n)`, shuffle, choice. |
| `uuid` | *none* | v4 and v7, parse, format, compare. The entropy is a parameter. |
| `path` | *none* | Join, normalise, split, containment. Touches no disk. |
| `url` | *none* | Parse, build, resolve, percent code, query strings. |
| `log` | `io`, `Log` | Logging as an **effect**, and five sinks. |
| `csv` | *none* | Quotes, embedded newlines, headers, any separator. |
| `cli` | *none* | Flags, bundles, subcommands, `--`, a generated help page. |
| `testing` | *none* | Cases as values, table-driven suites, a golden diff that says where. |
| `toml` | *none* | Tables, arrays, inline tables, dotted paths. |
| `resilience` | *none* | Backoff with full jitter, circuit breaker, token bucket, deadlines. |
| `cache` | *none* | `Cache[V]`, least-recently-used with an expiry. |
| `crypto_hash` | *none* | SHA-256, HMAC-SHA-256, PBKDF2, constant-time compare. |
| `regex` | *none* | Thompson NFA — linear in the input, for every pattern. |
| `fs` | `io` | Walk, glob, copy, temporary paths. Path arithmetic is `path`. |
| `jwt` | *none* | HS256, and the algorithm always comes from the caller. |
| `cors` | *none* | A request in, the headers a response needs out. |
| `dotenv` | `io` | `.env` files — and it never touches the process environment. |

Three packages rather than one, and the split is the same argument each time:
a program that only needs to *build* a request never acquires the capability to
send one, and a program that only needs to route never acquires the capability
to listen.

## Why `http` and `http_server` are two packages

`http` cannot open a socket, and that is a fact the compiler checks rather than
a claim the README makes. Its manifest declares no capability; effects are
inferred from its source; a version of it that started dialling out would
exceed its ceiling and **fail to build in every consumer**.

So a program can parse and route HTTP while remaining provably unable to reach
the network. The capability lives in `http_server`, which a consumer has to
name to get.

In every other language, depending on "the HTTP library" means depending on
something that can reach the network, and the only defence is reading its
source. That is the difference this package system exists to make.

## Running the demos

```sh
grasm run packages/http-demo/src/main.gr
grasm run packages/http_server-demo/src/main.gr
grasm run packages/http_server-demo/src/main.gr -- serve   # over a real socket
grasm run packages/http_client-demo/src/main.gr
grasm run packages/http_client-demo/src/main.gr -- serve   # client and server, both Grasm
grasm run packages/json-demo/src/main.gr
grasm run packages/bytes-demo/src/main.gr
grasm run packages/time-demo/src/main.gr
grasm run packages/random-demo/src/main.gr
grasm run packages/uuid-demo/src/main.gr
grasm run packages/path-demo/src/main.gr
grasm run packages/url-demo/src/main.gr
grasm run packages/log-demo/src/main.gr
grasm run packages/csv-demo/src/main.gr
grasm run packages/cli-demo/src/main.gr
grasm run packages/testing-demo/src/main.gr
grasm run packages/toml-demo/src/main.gr
grasm run packages/resilience-demo/src/main.gr
grasm run packages/cache-demo/src/main.gr
grasm run packages/crypto_hash-demo/src/main.gr
grasm run packages/regex-demo/src/main.gr
grasm run packages/fs-demo/src/main.gr
grasm run packages/jwt-demo/src/main.gr
grasm run packages/cors-demo/src/main.gr
grasm run packages/dotenv-demo/src/main.gr
```

## The pattern every one of these follows

**Split reach from computation.** `path` is arithmetic and declares nothing;
walking a directory is `fs`. `url` is text and declares nothing; dialling is
`http_client`. `time` is a calendar and declares nothing; reading the clock is
one `io` call in the consumer's `main`. `random` is a seeded generator and
declares nothing; entropy is `random_bytes`, which the consumer calls once.

**Prefer an effect to a capability.** `log` is the clearest case. A library
that logs by calling `print` declares `io`, and every consumer then depends on
something that can write anywhere. A library that logs by performing `Log`
declares `Log` — see `packages/log-demo`, whose `work` function logs from three
calls deep with the row `/ {Log}` and therefore cannot print, cannot open a
file, cannot reach the network. What logging *means* is decided by the line
that installs the handler.

**Every package prefixes its own type names.** `JsonError`, `RegexError`,
`CsvError`, `TomlError`, `DotenvError`, `JwtError`, `CacheEntry`, `FsEntry`.
Types share one namespace across the whole program ([T-MODULE-NS]), so two
packages that each declare a `Bad` cannot both be imported — which this
repository discovered the moment it had eight libraries (F78). The convention
has no enforcement; the compiler at least now names both modules when it
refuses.

**Every refusal is a value.** A URL, a UUID, a timestamp, a base64 string is
input somebody else chose. Not one decoder here panics.

The socket path is opt-in. The corpus does not open sockets by default: a gate
that depends on the loopback network fails for reasons that have nothing to do
with the language, and a gate nobody trusts is a gate nobody reads. Project 21
draws the same line for the same reason.
