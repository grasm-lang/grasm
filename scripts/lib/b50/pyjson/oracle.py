"""The oracle for packages/pyjson: CPython 3.13's json module (the C
accelerator). Text travels as code points in hex joined by '.', '-' when
empty, so that lone surrogates survive."""
import json
import struct
import sys


def cps(s):
    return ".".join("%x" % ord(c) for c in s) or "-"


def text(h):
    return "" if h == "-" else "".join(chr(int(x, 16)) for x in h.split("."))


def dump(v):
    if v is None:
        return "n"
    if v is True:
        return "t"
    if v is False:
        return "f"
    if isinstance(v, int):
        return "i" + str(v)
    if isinstance(v, float):
        return "d%x" % struct.unpack("<Q", struct.pack("<d", v))[0]
    if isinstance(v, str):
        return "s" + cps(v)
    if isinstance(v, list):
        return "[" + ",".join(dump(x) for x in v) + "]"
    return "{" + ",".join("s" + cps(k) + "=" + dump(x) for k, x in v.items()) + "}"


SEPARATORS = {"0": None, "1": (",", ":"), "2": (",", ": "), "3": (" , ", " :  ")}


def options(flags, indent):
    return dict(ensure_ascii=flags[0] == "1", allow_nan=flags[1] == "1", sort_keys=flags[2] == "1", separators=SEPARATORS[flags[3]],
                indent=None if indent == "~" else int(indent[1:]) if indent[0] == "i" else text(indent[1:]))


def dumped(v, **kw):
    try:
        return cps(json.dumps(v, **kw))
    except ValueError as e:
        return "V" + str(e).encode().hex()


def line(f):
    op = f[0]
    if op == "lo":
        try:
            return dump(json.loads(text(f[1])))
        except json.JSONDecodeError as e:
            return "E" + str(e).encode().hex()
        except ValueError as e:
            return "V" + str(e).encode().hex()
    if op == "du":
        try:
            v = json.loads(text(f[1]))
        except ValueError:
            return "X"
        return dumped(v, **options(f[2], f[3]))
    if op == "df":
        return dumped(struct.unpack("<d", struct.pack("<Q", int(f[1], 16)))[0], allow_nan=f[2] == "1")
    if op == "ds":
        return dumped(text(f[1]), ensure_ascii=f[2] == "1")
    # The input itself may pass the limit; only dumps is held to it.
    sys.set_int_max_str_digits(0)
    n = int(f[1])
    sys.set_int_max_str_digits(4300)
    return dumped(n)


sys.set_int_max_str_digits(4300)
out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
