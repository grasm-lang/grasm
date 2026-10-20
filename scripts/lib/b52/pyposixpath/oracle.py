"""The oracle for packages/pyposixpath: CPython 3.13's posixpath, with
os.getcwd() replaced by the cwd each line gives. Text is hex UTF-8, '-'
when empty; lists are joined by ','."""
import os
import posixpath
import sys


def text(h):
    return "" if h == "-" else bytes.fromhex(h).decode()


def hx(s):
    return s.encode().hex() or "-"


def lst(h):
    return [] if h == "~" else [text(x) for x in h.split(",")]


def line(f):
    cwd = text(f[1])
    os.getcwd = lambda: cwd
    op = f[0]
    try:
        if op == "np":
            return hx(posixpath.normpath(text(f[2])))
        if op == "jo":
            parts = lst(f[2])
            return hx(posixpath.join(*parts)) if parts else "~"
        if op == "sp":
            return " ".join(hx(x) for x in posixpath.split(text(f[2])))
        if op == "se":
            return " ".join(hx(x) for x in posixpath.splitext(text(f[2])))
        if op == "sr":
            return " ".join(hx(x) for x in posixpath.splitroot(text(f[2])))
        if op == "bn":
            return hx(posixpath.basename(text(f[2])))
        if op == "dn":
            return hx(posixpath.dirname(text(f[2])))
        if op == "ia":
            return str(posixpath.isabs(text(f[2])))
        if op == "cp":
            return hx(posixpath.commonprefix(lst(f[2])))
        if op == "cm":
            return hx(posixpath.commonpath(lst(f[2])))
        if op == "ab":
            return hx(posixpath.abspath(text(f[2])))
        return hx(posixpath.relpath(text(f[2]), text(f[3])))
    except ValueError as e:
        return "E" + hx(str(e))


out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
