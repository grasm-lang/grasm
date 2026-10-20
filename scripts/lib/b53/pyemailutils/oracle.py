# The oracle for packages/pyemailutils: CPython 3.13's email.utils. Text is
# UTF-8 in hex, '-' when empty.
import sys
from email import utils


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def hx(s):
    return s.encode("utf-8").hex() or "-"


def pair(p):
    return hx(p[0]) + "|" + hx(p[1])


def line(f):
    op = f[0]
    if op == "pa":
        return pair(utils.parseaddr(unhex(f[2]), strict=f[1] == "1"))
    if op == "ga":
        return ",".join(pair(p) for p in utils.getaddresses([unhex(x) for x in f[2:]], strict=f[1] == "1")) or "-"
    if op == "fa":
        try:
            return hx(utils.formataddr((unhex(f[1]), unhex(f[2]))))
        except UnicodeEncodeError as e:
            return "E:" + hx(str(e))
    if op == "qu":
        return hx(utils.quote(unhex(f[1])))
    return hx(utils.unquote(unhex(f[1])))


out = []
for l in sys.stdin:
    l = l.rstrip("\n")
    if l:
        out.append(line(l.split(" ")))
sys.stdout.write("".join(o + "\n" for o in out))
