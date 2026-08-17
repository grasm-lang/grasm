# 21 — Single-Threaded HTTP Server
**Tests:** TCP sockets, request parsing, response framing, the `net` capability.
**Result:** works. Parser gated across three engines; live server verified with `curl`.

## The split, and why

The program is two halves and they are separated deliberately. The **parsing and
framing** is pure and is exercised on every run, so it can be compared across all
three engines. The **socket loop** needs a peer and cannot be part of an
automated comparison, so it runs only on demand:

```
grasm run main.gr -- serve      # then http://localhost:8971
```

A server whose parser is untested is wrong in a way nobody sees until a real
browser sends something slightly unusual — which is why the malformed cases are
the bulk of the file. Nine of them, each answering 400 rather than crashing:
empty, whitespace, two parts, four parts, no version, unknown method, relative
target, header without a colon, lower-case method.

## Verified live

```
$ curl -s localhost:8971/                       -> It works.
$ curl -s "localhost:8971/hello?name=ada%20lovelace"  -> hello, ada lovelace
$ curl -o /dev/null -w '%{http_code}' .../nope  -> 404
$ curl -X POST -o /dev/null -w '%{http_code}' / -> 405
$ curl -I localhost:8971/                       -> 200, Content-Length: 87, no body
```

Percent-decoding is included because a browser sends `%20` and a server that
skips it reports the wrong value with no error anywhere. Ten decode cases,
including the malformed ones (`%`, `%4`, `%zz`) that must pass through rather
than read off the end.

`Content-Length` is computed from the body, never guessed, and checked against
the actual body on every response — except HEAD, which is *supposed* to declare
a length and send nothing, and is checked for exactly that instead.

`main` declares `/ {io, net}`. A package that never asked for `net` could not
contain this program ([T-CAPABILITY]).

## Findings
None. This is the project the `net` capability was built for, and it did not
need anything further.
