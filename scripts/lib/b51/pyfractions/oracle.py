"""The oracle for packages/pyfractions: CPython 3.13's fractions.Fraction.
Fractions travel as n/d; text as hex UTF-8, '-' when empty."""
import struct
import sys
from fractions import Fraction


def text(h):
    return "" if h == "-" else bytes.fromhex(h).decode()


def hx(s):
    return s.encode().hex() or "-"


def fr(t):
    n, d = t.split("/")
    return Fraction(int(n), int(d))


def show(f):
    return f"{f.numerator}/{f.denominator}"


def line(f):
    op = f[0]
    try:
        if op == "fs":
            return show(Fraction(text(f[1])))
        if op == "ff":
            return show(Fraction(struct.unpack("<d", struct.pack("<Q", int(f[1], 16)))[0]))
        if op == "ar":
            a, k = fr(f[2]), f[1]
            if k == "pow":
                return show(a ** int(f[3]))
            b = fr(f[3])
            if k == "fdiv":
                return str(a // b)
            ops = {"add": lambda: a + b, "sub": lambda: a - b, "mul": lambda: a * b, "div": lambda: a / b, "mod": lambda: a % b}
            return show(ops[k]())
        if op == "ld":
            return show(fr(f[1]).limit_denominator(int(f[2])))
        if op == "rd":
            a = fr(f[1])
            return str(round(a)) if f[2] == "~" else show(round(a, int(f[2])))
        if op == "fl":
            a = fr(f[1])
            import math
            return "%x %d %d %d" % (struct.unpack("<Q", struct.pack("<d", float(a)))[0], math.floor(a), math.ceil(a), math.trunc(a))
        if op == "fm":
            return hx(format(fr(f[1]), text(f[2])))
        a = fr(f[1])
        return hx(str(a)) + " " + hx(repr(a))
    except (ValueError, ZeroDivisionError, OverflowError) as e:
        return "E" + type(e).__name__ + ":" + hx(str(e))


out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
