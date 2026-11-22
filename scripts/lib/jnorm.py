#!/usr/bin/env python3
"""Whether two journals record the same run (`scripts/clidiff.sh`, `journal`).

    jnorm.py <a.jrnl> <b.jrnl>

Exit 0 when they are the same bytes once the values of the built-ins whose
answer depends on when and where the run happened -- the clock, entropy,
ports and peers: the only ones that differed between two recordings of the
corpus by the Rust `grasm` (docs/self-hosting.md §6.22) -- are set aside;
2 when what differs is a failed name lookup's `IoFailed` text -- the Rust
interpreter's `std` says "failed to lookup address information: ...", and
both runtimes report the lookup as ENOENT, the code `getaddrinfo` gave
dropped, so the TIR interpreter's text is "No such file or directory (os
error 2)" (docs/known-issues.md; docs/self-hosting.md §6.23); 3 when what differs is only socket
handles numbered otherwise after a `net_close`; 1 otherwise, saying where.

The Rust interpreter never gives a closed socket's handle out again, where
the Rust runtime and the Grasm one take the lowest free (docs/known-issues.md,
S3; the coordinator's decision, docs/self-hosting.md §6.22). So after the
run's first `net_close`, a handle a `tcp_listen`, `tcp_listen_on`,
`tcp_connect`, `tcp_accept` or `udp_bind` answers may differ, and the
handles a `net_wait` answers with -- but only as one renumbering: each
handle the first journal (the Rust command's) gives out is new, and stands
for the second journal's handle from the record that made it on. Anything
else must be the same.
"""
import struct
import sys

VARY = {b"now_millis", b"monotonic_millis", b"monotonic_nanos", b"random_bytes", b"net_port", b"tcp_peer"}
MAKES = {b"tcp_listen", b"tcp_listen_on", b"tcp_connect", b"tcp_accept", b"udp_bind"}


def wire(raw, at):
    """The value at `at` (docs/replay.md's encoding) and where it ends."""
    tag = raw[at]
    at += 1
    if tag == 0:
        return ("unit",), at
    if tag in (1, 2):
        v = struct.unpack("<q", raw[at:at + 8])[0]
        return (("int" if tag == 1 else "float"), v), at + 8
    if tag == 3:
        return ("bool", raw[at]), at + 1
    if tag in (4, 5):
        n = struct.unpack("<I", raw[at:at + 4])[0]
        return ("text", raw[at + 4:at + 4 + n]), at + 4 + n
    if tag in (6, 7, 8):
        head = ()
        if tag == 8:
            for _ in range(2):
                n = struct.unpack("<I", raw[at:at + 4])[0]
                head += (raw[at + 4:at + 4 + n],)
                at += 4 + n
        n = struct.unpack("<I", raw[at:at + 4])[0]
        at += 4
        items = []
        for _ in range(n):
            v, at = wire(raw, at)
            items.append(v)
        return (("list", "tuple", "variant")[tag - 6],) + head + (items,), at
    raise ValueError("tag %d" % tag)


def ok_int(raw):
    """The handle an `Ok(handle)` holds, else None."""
    try:
        v, end = wire(raw, 0)
    except (ValueError, IndexError, struct.error):
        return None
    if end == len(raw) and v[0] == "variant" and v[2] == b"Ok" and len(v[3]) == 1 and v[3][0][0] == "int":
        return v[3][0][1]
    return None


def ok_ints(raw):
    """The handles an `Ok([handle, ...])` holds, else None."""
    try:
        v, end = wire(raw, 0)
    except (ValueError, IndexError, struct.error):
        return None
    if end != len(raw) or v[0] != "variant" or v[2] != b"Ok" or len(v[3]) != 1 or v[3][0][0] != "list":
        return None
    if any(x[0] != "int" for x in v[3][0][1]):
        return None
    return [x[1] for x in v[3][0][1]]


def records(raw):
    if raw[:10] != b"GRASMJRNL2":
        return None
    at, out = 10, []
    while at < len(raw):
        n = struct.unpack("<I", raw[at:at + 4])[0]
        name = raw[at + 4:at + 4 + n]
        at += 4 + n
        m = struct.unpack("<I", raw[at:at + 4])[0]
        out.append((name, raw[at + 4:at + 4 + m]))
        at += 4 + m
    return out


a = records(open(sys.argv[1], "rb").read())
b = records(open(sys.argv[2], "rb").read())
if a is None or b is None:
    print("not a journal")
    sys.exit(1)
if [x[0] for x in a] != [x[0] for x in b]:
    print("the calls differ: %d and %d records" % (len(a), len(b)))
    sys.exit(1)
closed = False
to_b, to_a = {}, {}
renumbered = 0


iofailed = None
for i, (x, y) in enumerate(zip(a, b)):
    name = x[0]
    if name in MAKES:
        h, k = ok_int(x[1]), ok_int(y[1])
        if h is not None and k is not None and (h == k or closed) and h not in to_b:
            # A socket made -- a handle the first journal, the Rust
            # interpreter's, never gave out before, as it gives none out
            # twice: from here on h of one journal is k of the other, and
            # whatever k stood for before was closed.
            to_a.pop(to_b.get(h), None)
            to_b.pop(to_a.get(k), None)
            to_b[h], to_a[k] = k, h
            if h != k:
                renumbered += 1
            continue
    if name == b"net_close":
        closed = True
    if x[1] == y[1] or name in VARY:
        continue
    if closed and name == b"net_wait":
        hs, ks = ok_ints(x[1]), ok_ints(y[1])
        if hs is not None and ks is not None and len(hs) == len(ks) and all(to_b.get(h) == k for h, k in zip(hs, ks)):
            renumbered += 1
            continue
    if b"failed to lookup address information: " in x[1] and y[1].endswith(b"No such file or directory (os error 2)") and x[1].split(b"IoFailed")[0] == y[1].split(b"IoFailed")[0]:
        iofailed = iofailed or "record %d (%s): a failed lookup's IoFailed text" % (i + 1, name.decode(errors="replace"))
        continue
    print("record %d (%s) differs" % (i + 1, name.decode(errors="replace")))
    sys.exit(1)
if iofailed:
    print(iofailed)
    sys.exit(2)
if renumbered:
    print("%d record(s) with socket handles numbered otherwise after a net_close" % renumbered)
    sys.exit(3)
sys.exit(0)
