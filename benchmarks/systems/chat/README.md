# chat — high-concurrency broadcast server

Many TCP clients, grouped into channels; every line a client sends is delivered
to every *other* client in its channel. Three servers with the same semantics
(Grasm, Rust/tokio, Go), one load generator (Go) used for all three.

```
./verify.sh      # 50 connections, 5 channels of 10, exact delivery check (seconds)
./run.sh         # full size: 10,000 connections (see knobs at the top of run.sh)
```

`run.sh` builds everything into `build/`, then for each server starts it, runs
the loadgen, stops it, and prints deliveries, expected deliveries, throughput
(deliveries/s), latency p50/p99/p999 and the server's peak RSS. It exits
non-zero unless every server delivered **exactly** the expected count with no
misrouted or self-echoed message.

## What it measures

- **Defaults:** `CONNS=10000`, `CHSIZE=1000` (so 10 channels; client *k* joins
  channel *k* mod 10), `SENDERS=CONNS/10` = 1,000 senders, `MSGS=10` each at
  `RATE=1` msg/s, spread evenly over the second. Each message fans out to 999
  peers: 1,000 × 10 × 999 = **9,990,000 deliveries** over ~10 s of sending.
- **Throughput** = deliveries / (last delivery − first send). Below saturation
  it equals the offered load (`SENDERS × RATE × (CHSIZE−1)`); raise `RATE` to
  find where a server stops keeping up — latency then grows with the backlog.
- **Latency** = receive time − send time stamped in the message, one
  monotonic clock (the loadgen process does both ends). Measured from the
  actual send, not the scheduled one, so it does not correct for coordinated
  omission when the loadgen itself falls behind.
- **Peak RSS** = the server pid's `ps -o rss=`, sampled every 100 ms by the
  loadgen (`-pid`), across join, send and drain.
- The loadgen runs on the same machine and competes for CPU with the servers;
  Rust and Go servers use all cores, Grasm uses one (see below).

## Protocol (deviation: TCP lines, not WebSocket)

A real chat server would speak WebSocket. Implementing the handshake and frame
masking three times is a lot of code that measures SHA-1 and base64, not
connection handling, so all three speak newline-delimited text over TCP:

```
client -> server   J <channel>\n           first line; server answers OK\n
client -> server   M <sender> <seq> <ns>\n  broadcast verbatim to the channel's others
```

The server never parses message lines. Each server reads a chunk, broadcasts
everything up to the last complete newline as **one write per peer**, and keeps
a trailing partial line for the next read (all three do the same, and verified
with a line split across three writes).

## The three servers

- **Grasm** (`grasm/server.gr`): one thread, one `net_wait` (poll(2)) loop over
  every handle; per-handle state in lists indexed by handle (handles are small
  reused integers). Built with `grasm build --native`. `TCP_NODELAY` on (the
  runtime sets it on every accepted stream).
- **Rust** (`rust/`): tokio multi-thread runtime, a task per connection reading
  and a writer task per connection fed by an unbounded `mpsc`
  (`tokio::sync::broadcast` would drop messages for a lagging receiver; the
  benchmark requires every delivery). `TCP_NODELAY` on.
- **Go** (`go/`): goroutine per connection plus a writer goroutine draining an
  unbounded queue with `writev` (`net.Buffers`). stdlib only. `TCP_NODELAY` on
  (Go's default).

### Why the Grasm server is an event loop, not tasks

`concurrent { go { … } }` tasks may call `tcp_accept`/`tcp_read`, but tasks are
cooperative: they switch only at channel operations, and a socket call blocks
the whole thread. This program prints `accepted`/`task1 read` *before*
`task2 running` — task 2 cannot run while task 1 sits in `tcp_accept`:

```
fn main() {
    let l = tcp_listen(0) ?? 0 - 1
    print("port {net_port(l) ?? 0}")
    let ch = chan_new(10)
    concurrent {
        go {
            let c = tcp_accept(l) ?? 0 - 1      // blocks the thread, not the task
            print("accepted {c}")
            print("task1 read {tcp_read(c) ?? ""}")
            chan_send(ch, 1)
        }
        go {
            print("task2 running")
            chan_send(ch, 2)
        }
    }
}
```

A task per connection would therefore serve one client at a time. `spawn` does
not carry `net` at all. `net_wait` is the documented way for one Grasm program
to hold many connections (`packages/mqttbroker`, `packages/wsserver` use it),
so the server is a single-threaded readiness loop.

### Known limitation of the Grasm server

`tcp_write`/`tcp_write_bytes` are blocking `write_all`s and `net_wait` only
reports *readable* handles, so a client that stops reading eventually blocks
the whole server once its socket buffer fills: no other client is served, and
new connections are not accepted. (Reproduced: one joined client that never
reads, a peer that floods 20 MB; a new client gets no `OK` within 3 s. The Go
server answers immediately, buffering in memory instead.) The loadgen reads
continuously, so the benchmark does not hit this, but a production server
would.

All three servers now run with Nagle's algorithm off. Grasm still cannot set
any socket option itself, but its runtime turns on `TCP_NODELAY` for every
accepted and dialed stream. Before that, the Grasm server kept Nagle on while
the Rust and Go servers turned it off.

## Connection count: why 10,000, and going higher

The brief asked for 100,000 connections. On this macOS machine that is not
possible to one `127.0.0.1` port:

- **Ephemeral ports:** 49152–65535, about 16k source ports per source address.
  Every client connection to `127.0.0.1` needs one.
- **File descriptors:** `kern.maxfilesperproc` is 61440 (whatever `ulimit -n`
  says), and the loadgen holds one fd per client too.
- **Listen backlog:** `kern.ipc.somaxconn` is 128; the loadgen dials 64 at a
  time with retries.

10,000 stays clear of all three, which is why it is the default. To go higher:

1. More source addresses: `sudo ifconfig lo0 alias 127.0.0.2` (…`.3`, …) and
   `LADDRS=127.0.0.1,127.0.0.2,…` — each adds ~16k ports. Alternatively widen
   the range: `sudo sysctl net.inet.ip.portrange.first=1024` (~64k ports).
   Extra *server* ports are the other route (the loadgen's `-addrs` takes a
   list; connection *i* dials entry *i* mod len), but whether macOS reuses an
   ephemeral port toward a different destination port was not verified here —
   Linux does; source aliases work on both. The servers take one port each. The Grasm server listens on
   `127.0.0.1` with `tcp_listen`. `tcp_listen_on(host, port)` can now bind
   other addresses, but the server does not take a host argument, so the
   aliases must be the *client* side.
2. File descriptors: `sudo sysctl kern.maxfiles=… kern.maxfilesperproc=…`, or
   split the clients over several loadgen processes.
3. Linux: `ulimit -n`, `net.ipv4.ip_local_port_range`, `net.core.somaxconn`.

At ~1,000 clients per channel the default run is dominated by fan-out, not by
the connection count; `CHSIZE` changes that balance.
