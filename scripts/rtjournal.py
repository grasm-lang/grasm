#!/usr/bin/env python3
"""Journals written by hand, for `scripts/fuzz_rtsys.sh` (docs/self-hosting.md §6.12).

    rtjournal.py <seed> <out.jrnl>

Writes a journal in the format `crates/grasm_journal` reads -- `GRASMJRNL2`,
then per record a `u32` name length, the name, a `u32` value length, the
value -- for `compiler/rtsys/jfuzz.gr`, whose script of calls is the
journal's first record (its `args`). Each later record answers one call of
the script with a value of the shape that call answers, drawn at random:
text of every length either side of the inline limit, Unicode, bytes that are
not UTF-8 (read back lossily), lists, tuples, `Ok`, `Err`, `Some`, `None`,
`IoError`s, integers across the whole range. Some journals are broken on
purpose, each in one way a replay must refuse: too few records, a record
naming another call, a name that is not UTF-8, a file that is not a journal,
one that ends mid-record or mid-value, a value with bytes after it, an
unknown tag -- or the script makes a C call, which a replay refuses. Prints
what kind of journal it wrote.
"""
import random
import struct
import sys

MAGIC = b"GRASMJRNL2"


def u32(n):
    return struct.pack("<I", n)


def w_unit():
    return b"\x00"


def w_int(v):
    return b"\x01" + struct.pack("<q", v)


def w_float(v):
    return b"\x02" + struct.pack("<d", v)


def w_bool(b):
    return b"\x03" + (b"\x01" if b else b"\x00")


def w_str(raw):
    return b"\x04" + u32(len(raw)) + raw


def w_bytes(raw):
    return b"\x05" + u32(len(raw)) + raw


def w_list(items):
    return b"\x06" + u32(len(items)) + b"".join(items)


def w_tuple(items):
    return b"\x07" + u32(len(items)) + b"".join(items)


def w_variant(ty, name, fields):
    return b"\x08" + u32(len(ty)) + ty + u32(len(name)) + name + u32(len(fields)) + b"".join(fields)


def ok(v):
    return w_variant(b"!", b"Ok", [v])


def err(v):
    return w_variant(b"!", b"Err", [v])


def io_error(r):
    kind = r.choice([b"NotFound", b"IoFailed"])
    return err(w_variant(b"IoError", kind, [w_str(text(r))]))


def text(r):
    n = r.choice([0, 1, 5, 37, 38, 39, 60, r.randrange(0, 120)])
    out = bytearray()
    while len(out) < n:
        k = r.randrange(10)
        if k == 0:
            out += "é".encode()
        elif k == 1:
            out += "🙂".encode()
        elif k == 2:
            out += bytes([r.choice([0xFF, 0xC3, 0xE2, 0xF0, 0x80])])
        elif k == 3:
            out += b"\n"
        else:
            out += bytes([r.randrange(0x61, 0x7B)])
    return bytes(out)


def any_int(r):
    return r.choice([0, 1, -1, 2**63 - 1, -(2**63), r.randrange(-(2**63), 2**63), r.randrange(0, 2**41)])


# What each letter calls, and a value of the shape it answers.
CALLS = {
    "n": ("now_millis", lambda r: w_int(any_int(r))),
    "m": ("monotonic_millis", lambda r: w_int(any_int(r))),
    "N": ("monotonic_nanos", lambda r: w_int(any_int(r))),
    "s": ("sleep_millis", lambda r: r.choice([w_unit(), w_int(3), w_str(b"x")])),
    "e": ("file_exists", lambda r: r.choice([w_bool(True), w_bool(False), w_int(1), w_unit()])),
    "r": ("read_file", lambda r: r.choice([ok(w_str(text(r))), io_error(r)])),
    "b": ("read_bytes", lambda r: r.choice([ok(w_bytes(text(r))), io_error(r)])),
    "l": ("list_dir", lambda r: r.choice([ok(w_list([w_str(text(r)) for _ in range(r.randrange(0, 7))])), io_error(r)])),
    "E": ("env", lambda r: r.choice([w_variant(b"Option", b"Some", [w_str(text(r))]), w_variant(b"Option", b"None", [])])),
    "i": ("input", lambda r: w_str(text(r))),
    "R": ("random_bytes", lambda r: r.choice([ok(w_bytes(bytes(r.randrange(256) for _ in range(4)))), io_error(r)])),
    "c": ("net_close", lambda r: r.choice([w_unit(), w_int(0)])),
    "w": ("net_wait", lambda r: r.choice([ok(w_list([w_int(r.randrange(0, 9)) for _ in range(r.randrange(0, 4))])), io_error(r)])),
    "p": ("proc_run", lambda r: r.choice([ok(w_tuple([w_int(r.randrange(-1, 256)), w_str(text(r)), w_str(text(r))])), io_error(r)])),
    "a": ("args", lambda r: w_list([w_str(text(r)) for _ in range(r.randrange(0, 4))])),
    "z": ("file_size", lambda r: r.choice([ok(w_int(any_int(r))), io_error(r)])),
    "t": ("tcp_read", lambda r: r.choice([ok(w_str(text(r))), io_error(r)])),
    "W": ("write_file", lambda r: r.choice([ok(w_unit()), io_error(r)])),
}


def record(name, value):
    return u32(len(name)) + name + u32(len(value)) + value


def main():
    seed, out = int(sys.argv[1]), sys.argv[2]
    r = random.Random(seed)
    letters = list(CALLS)
    script = "".join(r.choice(letters) for _ in range(r.randrange(1, 40)))
    kind = r.choice(["whole"] * 6 + ["short", "renamed", "badname", "notjournal", "midrecord", "midvalue", "trailing", "badtag", "ccall", "empty"])
    if kind == "ccall":
        at = r.randrange(0, len(script) + 1)
        script = script[:at] + "C" + script[at:]
    recs = [(b"args", w_list([w_str(script.encode())]))]
    for c in script:
        if c == "C":
            continue
        name, make = CALLS[c]
        recs.append((name.encode(), make(r)))
    if kind == "short":
        recs = recs[: r.randrange(1, len(recs) + 1)] if len(recs) > 1 else recs
        if len(recs) == len([c for c in script if c != "C"]) + 1:
            recs = recs[:-1]
    elif kind == "renamed" and len(recs) > 1:
        i = r.randrange(1, len(recs))
        recs[i] = (r.choice([b"read_file", b"now_millis", b"bogus_call"]), recs[i][1])
    elif kind == "badname" and len(recs) > 1:
        i = r.randrange(1, len(recs))
        recs[i] = (b"\xffname\xc3", recs[i][1])
    elif kind == "empty":
        recs = []
    body = MAGIC + b"".join(record(n, v) for n, v in recs)
    if kind == "notjournal":
        body = r.choice([b"", b"GRASMJRNL1", b"GRASMJRN", b"hello world"]) + body[10:]
    elif kind == "midrecord":
        body = body[: r.randrange(len(MAGIC) + 1, len(body))] if len(body) > len(MAGIC) + 1 else body + b"\x05"
    elif kind == "midvalue" and len(recs) > 1:
        # A value cut short inside its record: the record's length agrees.
        n, v = recs[-1]
        cut = v[: r.randrange(0, len(v))]
        body = MAGIC + b"".join(record(a, b) for a, b in recs[:-1]) + record(n, cut)
    elif kind == "trailing" and len(recs) > 1:
        n, v = recs[-1]
        body = MAGIC + b"".join(record(a, b) for a, b in recs[:-1]) + record(n, v + b"\x00")
    elif kind == "badtag" and len(recs) > 1:
        n, v = recs[-1]
        body = MAGIC + b"".join(record(a, b) for a, b in recs[:-1]) + record(n, bytes([r.randrange(9, 256)]) + v[1:])
    with open(out, "wb") as f:
        f.write(body)
    print(kind)


if __name__ == "__main__":
    main()
