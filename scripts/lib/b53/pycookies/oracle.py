# The oracle for packages/pycookies: CPython 3.13's http.cookies. Text is
# UTF-8 in hex, '-' when empty. A line loads a header ('ld <hex>') or sets
# values and attributes ('st <key> <value> [<key> <attr> <value|~>]...').
import sys
from http import cookies


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def hx(s):
    return s.encode("utf-8").hex() or "-"


def state(c, err):
    vals = ",".join(hx(k) + "=" + hx(c[k].value) for k in sorted(c.keys())) or "-"
    return (err or "-") + " " + hx(c.output()) + " " + vals


def line(f):
    c = cookies.SimpleCookie()
    err = None
    try:
        if f[0] == "ld":
            c.load(unhex(f[1]))
        else:
            c[unhex(f[1])] = unhex(f[2])
            rest = f[3:]
            for i in range(0, len(rest), 3):
                c[unhex(rest[i])][unhex(rest[i + 1])] = None if rest[i + 2] == "~" else unhex(rest[i + 2])
    except cookies.CookieError as e:
        err = "E:" + hx(str(e))
    except KeyError as e:
        err = "K:" + hx(str(e))
    return state(c, err)


out = []
for l in sys.stdin:
    l = l.rstrip("\n")
    if l:
        out.append(line(l.split(" ")))
sys.stdout.write("".join(o + "\n" for o in out))
