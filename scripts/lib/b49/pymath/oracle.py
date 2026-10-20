"""The oracle for packages/pymath: Python 3.13's math. Floats are given and
answered as the hex of their bits; lists are joined by `/`."""
import math
import struct
import sys


def f(h):
    return struct.unpack("<d", struct.pack("<Q", int(h, 16)))[0]


def fh(x):
    return "%x" % struct.unpack("<Q", struct.pack("<d", x))[0]


def fl(t):
    return [f(x) for x in t.split("/")] if t else []


def run(op, a):
    if op == "fsum":
        return fh(math.fsum(fl(a[0])))
    if op == "sumprod":
        return fh(float(math.sumprod(fl(a[0]), fl(a[1]))))
    if op == "prod":
        return fh(float(math.prod(fl(a[0]))))
    if op == "fma":
        return fh(math.fma(f(a[0]), f(a[1]), f(a[2])))
    if op in ("lgamma", "gamma", "ulp"):
        return fh(getattr(math, op)(f(a[0])))
    if op == "hypot":
        return fh(math.hypot(*fl(a[0])))
    if op == "dist":
        return fh(math.dist(fl(a[0]), fl(a[1])))
    if op == "remainder":
        return fh(math.remainder(f(a[0]), f(a[1])))
    if op == "nextafter":
        return fh(math.nextafter(f(a[0]), f(a[1]), steps=None if a[2] == "~" else int(a[2])))
    if op == "isqrt":
        return str(math.isqrt(int(a[0])))
    if op == "comb":
        return str(math.comb(int(a[0]), int(a[1])))
    if op == "perm":
        return str(math.perm(int(a[0]), None if a[1] == "~" else int(a[1])))
    if op == "factorial":
        return str(math.factorial(int(a[0])))
    ints = [int(x) for x in a[0].split("/")] if a[0] else []
    return str(math.gcd(*ints)) if op == "gcd" else str(math.lcm(*ints))


out = []
for line in sys.stdin.read().splitlines():
    if not line:
        continue
    op, *a = line.split(" ")
    try:
        out.append(run(op, a))
    except ValueError as e:
        out.append("V" + str(e).encode().hex())
    except OverflowError as e:
        out.append("O" + str(e).encode().hex())
print("\n".join(out))
