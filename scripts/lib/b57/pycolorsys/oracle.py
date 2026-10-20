"""The oracle for pycolorsys: CPython 3.13's colorsys, called directly.
Floats travel as the hex of their IEEE bits and come back as repr."""
import colorsys
import struct
import sys

assert sys.version_info[:2] == (3, 13), sys.version


def f(h):
    return struct.unpack(">d", bytes.fromhex(h))[0]


def line(p):
    try:
        out = getattr(colorsys, p[0])(f(p[1]), f(p[2]), f(p[3]))
        return ",".join(repr(x) for x in out)
    except (ZeroDivisionError, OverflowError, ValueError) as e:
        return f"E:{type(e).__name__}:{e}"


out = [line(l.split(" ")[1:]) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
