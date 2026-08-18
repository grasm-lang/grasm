# 25 — DNS Client Resolver — **works**

**Tests:** UDP sockets, binary packet construction, bit shifting.
**Result:** builds and parses real DNS packets, and resolves against a real
server.

## What it does

The default run is self-contained and deterministic. The program binds two UDP
sockets, sends *itself* a real query, answers it with a real response, and
parses that back:

```
--- a query on the wire ---
query is 29 bytes
header:   12 34 01 00 00 01 00 00 00 00 00 00
question: 07 65 78 61 6d 70 6c 65 03 63 6f 6d 00 00 01 00 01
--- the exchange ---
client sent 29 bytes
server sees 1 socket ready
server got a query for example.com, id 0x1234, rd true
server sent 45 bytes
id 0x1234 response true NOERROR questions 1 answers 1
  example.com -> 93.184.216.34  ttl 300
```

Nothing is stubbed — those are the bytes this code built, over a real socket, so
all three engines run the encoder and the parser on every gate run.

It is also a real resolver:

```
$ grasm run main.gr -- ask 8.8.8.8 example.com
id 0x4321 response true NOERROR questions 1 answers 2
  example.com -> 104.20.23.154  ttl 300
  example.com -> 172.66.147.243  ttl 300
```

That form is not part of the automated comparison, for the same reason project
21's socket loop is not: its answer depends on the internet.

## Why it was blocked, and what changed

Three gaps, two of them each sufficient:

1. ~~**No byte type** (F9)~~ — `bytes` shipped as [L-LIB-BYTES]. A DNS query is
   a packed binary datagram and `str` re-encodes everything above 127.
2. ~~**No UDP** (F12)~~ — `udp_bind`, `udp_send` and `udp_recv` shipped as
   [L-LIB-UDP]. Datagram handles share the socket table with stream handles, so
   `net_wait` works on them and the resolver times out instead of hanging on a
   server that never answers.
3. ~~**No `tcp_connect`** (F11)~~ — shipped earlier.

## What writing it found

**A native-backend crash (S1).** `borrowed_read` decides which runtime function
to call in one table and what type it returns in another, and the second ended
in a catch-all that typed every unlisted `str` method as returning a string.
`byte_len` returns an `i64`. The compiled program called `.len()` on the integer
10 as though it were a pointer and took SIGSEGV, while `grasm run` printed the
right answer — a crash and an engine disagreement from one missing table row.
The catch-all is gone; every method is named, and an unlisted one now falls
through to the slower owning path instead of miscompiling.

**Two smaller gaps, worked around in the program.** A `.concat(...)` chain
cannot be split across lines, because a statement ends at the newline and a
leading `.` starts a new one — so building a packet needs a `join([...])` helper
or one very long line. And there is no way to format an integer in hex: a
`bytes` prints as hex and an `i64` prints as decimal, with no radix option, so
printing a packet ID the way every other DNS tool prints it needs a
hand-written `hex_digit`. Both are recorded as findings.

## What it did not need

The parsing — length-prefixed labels, compression pointers, the answer section —
needed nothing the language lacks. Following a pointer without advancing the
caller's cursor past it, which is the bug that makes a resolver read a record
type out of the middle of a hostname, is four lines.
