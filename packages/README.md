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
```

The socket path is opt-in. The corpus does not open sockets by default: a gate
that depends on the loopback network fails for reasons that have nothing to do
with the language, and a gate nobody trusts is a gate nobody reads. Project 21
draws the same line for the same reason.
