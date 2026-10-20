"""The oracle for packages/pyglob: CPython 3.13's glob.translate, escape and
has_magic. Text is hex UTF-8, '-' when empty."""
import glob
import sys


def text(h):
    return "" if h == "-" else bytes.fromhex(h).decode()


def hx(s):
    return s.encode().hex() or "-"


def line(f):
    if f[0] == "tr":
        return hx(glob.translate(text(f[1]), recursive=f[2] == "1", include_hidden=f[3] == "1", seps=text(f[4]) or None))
    if f[0] == "es":
        return hx(glob.escape(text(f[1])))
    return str(glob.has_magic(text(f[1])))


out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
