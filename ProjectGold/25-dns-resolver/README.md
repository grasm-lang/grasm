# 25 — DNS Client Resolver — **blocked**

**Tests:** UDP sockets, binary packet construction, bit shifting. **Result:**
cannot be written today.

## Why

Three gaps, and the first two are each sufficient:

1. **No byte type** (F9). A DNS query is a packed binary datagram — a 12-byte
   header with bit-level flag fields, then length-prefixed labels. `str` is
   UTF-8 text and there is nothing else to build it from.
2. **No UDP** (F12). Only TCP exists. DNS over TCP is legal and is not what the
   project asks for.
3. **No `tcp_connect`** (F11), so even the TCP fallback could not dial 8.8.8.8.

Bit manipulation is *no longer* a blocker — [L-LIB-BITS] landed for project 27,
and the header's `QR`, `OPCODE`, `RD` and `RCODE` fields would be
straightforward now.

## What would unblock it

A byte type with `udp_send`/`udp_recv`, and `tcp_connect` for the fallback. The
parsing itself — length-prefixed labels, compression pointers, the answer
section — needs nothing the language lacks; project 13's JSON parser is a harder
recursive-descent problem and it works.
