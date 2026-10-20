"""The oracle for packages/pycharclass: CPython 3.13's own str methods,
unicodedata and re, asked directly rather than through any table."""
import re
import sys
import unicodedata

CATS = ["Lu", "Ll", "Lt", "Lm", "Lo", "Mn", "Mc", "Me", "Nd", "Nl", "No", "Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po", "Sm", "Sc", "Sk", "So", "Zs", "Zl",
        "Zp", "Cc", "Cf", "Cs", "Co", "Cn"]
W, D, SP = re.compile(r"\w"), re.compile(r"\d"), re.compile(r"\s")


def text(h):
    return "" if h == "-" else "".join(chr(int(x, 16)) for x in h.split("."))


def char_code(c):
    tests = [c.isalpha(), c.isdecimal(), c.isdigit(), c.isnumeric(), c.isspace(), c.isprintable(), c.isupper(), c.islower(),
             unicodedata.category(c) == "Lt", c.isidentifier(), ("a" + c).isidentifier(), c.isalnum(), bool(W.match(c)), bool(D.match(c)),
             bool(SP.match(c))]
    return unicodedata.category(c) + "".join("1" if t else "0" for t in tests) + str(unicodedata.decimal(c, -1))


def line(f):
    if f[0] == "rg":
        lo, hi = int(f[1], 16), int(f[2], 16)
        runs, last = [], None
        for cp in range(lo, hi):
            v = char_code(chr(cp))
            if v != last:
                runs.append(f"{cp:x}:{v}")
                last = v
        return ",".join(runs)
    s = text(f[1])
    return "".join("1" if t else "0" for t in [s.isalpha(), s.isalnum(), s.isdecimal(), s.isdigit(), s.isnumeric(), s.isspace(), s.isprintable(),
                                                   s.isupper(), s.islower(), s.istitle(), s.isidentifier(), s.isascii()])


out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
