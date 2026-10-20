"""The oracle for pyreprlib: CPython 3.13's reprlib.Repr on a value read
with ast.literal_eval. Answers are the hex of their UTF-8."""
import ast
import reprlib
import sys

assert sys.version_info[:2] == (3, 13), sys.version


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def line(f):
    value = ast.literal_eval(unhex(f[0]))
    lim = [int(x) for x in f[1:9]]
    ind = f[10]
    indent = None if ind == "N" else (int(ind[1:]) if ind[0] == "i" else unhex(ind[1:]))
    r = reprlib.Repr(maxlevel=lim[0], maxtuple=lim[1], maxlist=lim[2], maxdict=lim[3], maxset=lim[4], maxstring=lim[5], maxlong=lim[6],
                     maxother=lim[7], fillvalue=unhex(f[9]), indent=indent)
    try:
        return r.repr(value)
    except ValueError as e:
        return f"E:ValueError:{e}"


out = [line(l.split(" ")[1:]).encode("utf-8").hex() or "-" for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
