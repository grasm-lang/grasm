"""The oracle for packages/pyfnmatch: CPython 3.13's fnmatch. Text is
code points in hex joined by '.', '-' when empty."""
import fnmatch
import sys
import warnings

warnings.simplefilter("ignore")


def text(h):
    return "" if h == "-" else "".join(chr(int(x, 16)) for x in h.split("."))


def cps(s):
    return ".".join("%x" % ord(c) for c in s) or "-"


def line(f):
    if f[0] == "tr":
        return cps(fnmatch.translate(text(f[1])))
    if f[0] == "mc":
        return "1" if fnmatch.fnmatchcase(text(f[1]), text(f[2])) else "0"
    names = [text(x) for x in f[1].split("/")]
    return ",".join(str(names.index(n)) for n in fnmatch.filter(names, text(f[2]))) or "~"


out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
