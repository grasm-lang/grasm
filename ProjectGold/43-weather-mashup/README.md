# 43 — Weather Mashup  *(phase 2, project 10)*

**Tests:** concurrent fan-out to several upstreams, a merge that survives a
partial answer, and a cache.
**Result:** works, and is **the one project of this phase still degraded**. The
reason is below and no amount of engineering in this repository fixes it.

```
grasm run main.gr     # the whole exchange: two upstreams and a mashup
```

## The degradation, stated first

**Grasm has no TLS.** It can dial `http://` and not `https://`, which is most
public APIs. That was true when the phase 2 plan was written and it is still
true; `net_wait` and `tcp_read_bytes` closed the other two limitations of this
phase and neither touches this one.

So the upstreams here are stand-ins, served by this same program on their own
ports. That is a real limitation in the smallest possible costume: the client
half is exactly what a real one would be — dial, send a request, read a
response, parse the JSON — and the only untrue thing about it is where it
points. Change two port numbers to a hostname and port 80 and it is a real
mashup of a real service, provided that service is willing to be spoken to in
plaintext.

Closing this needs a TLS implementation. That is not a weekend: certificate
chains, a modern cipher suite, and constant-time arithmetic that Grasm has no
way to promise. It is the largest single thing standing between this language
and ordinary web work, and it is honest to say so rather than to build a project
that pretends otherwise.

## What is real, and is the point

**Two upstreams are fetched concurrently.** Both requests are on the wire before
either reply is read, so the wait is the slower of the two rather than their
sum. A blocking client cannot have that shape.

It is also why this program can be its own upstream. All three servers and the
client run in one `net_wait` loop — the mashup's outbound request has to be
accepted by a listener in the same loop that is waiting for the mashup's reply,
and a blocking client would hang there, waiting for a server that cannot accept
until the client stops waiting.

**A partial answer degrades the page rather than failing it.**

```
lisbon: {"city": "lisbon", "temp_c": 22, "conditions": "clear", "aqi": 31, "missing": []}
cairo:  {"city": "cairo",  "temp_c": 0,  "conditions": "dust",  "aqi": 0,
         "missing": ["weather.temp_c", "air.aqi"]}
```

`cairo`'s weather service knows the city but not the temperature, and its air
service does not answer at all. The page still shows what was found and says
what was not — a mashup whose weather is missing because the air-quality service
is down is worse than no mashup, and one that silently reports `0` is worse
again.

The upstream is somebody else's service, so every field may be absent, the wrong
type, or not JSON at all. The harness checks all four cases, and they are the
same thing to the merge: a value it does not have.

**Answers are cached.** Two people asking about the same city a second apart
should not be two round trips, and the fourth request in the run proves it —
`lisbon: from cache, no round trip`.

## The honest limits

* **No TLS**, as above. Everything else here is downstream of that.
* **Fixed upstream data.** A mashup's test should fail when the mashup changes
  and not when the weather does.
* **The cache never expires.** A real one has a TTL, which needs a clock, which
  would make the harness's output depend on how fast the machine is.
* **One thread.**
