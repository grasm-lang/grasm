"""The oracle for pyhttpheaders: CPython 3.13's urllib.request and
http.cookiejar header helpers, called directly. The current year that
cookiejar reads for two-digit years is set per line (cookiejar's `time`
is replaced by a stand-in answering that year)."""
import http.cookiejar as cj
import sys
import types
import urllib.request as ur

assert sys.version_info[:2] == (3, 13), sys.version


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def hx(t):
    return t.encode("utf-8").hex() or "-"


def val(v):
    if v is None:
        return "N"
    if isinstance(v, str):
        return "s" + hx(v)
    if isinstance(v, float):
        assert v.is_integer()
        return "f" + str(int(v))
    return "i" + str(v)


def set_year(y):
    cj.time = types.SimpleNamespace(time=lambda: 0.0, localtime=lambda t=None: (y,))


def pairs_arg(a):
    lists = []
    for group in ([] if a == "-" else a.split("/")):
        pairs = []
        for p in ([] if group == "~" else group.split(",")):
            k, v = p.split(":")
            pairs.append((unhex(k), None if v == "N" else unhex(v[1:])))
        lists.append(pairs)
    return lists


def answer(f):
    op = f[0]
    if op == "L":
        return ",".join(hx(x) for x in ur.parse_http_list(unhex(f[1])))
    if op == "K":
        return ",".join(hx(k) + ":" + hx(v) for k, v in ur.parse_keqv_list([unhex(x) for x in f[1:]]).items())
    if op == "S":
        return ";".join(",".join(hx(k) + ":" + val(v) for k, v in pairs) for pairs in cj.split_header_words([unhex(x) for x in f[1:]]))
    if op == "J":
        return hx(cj.join_header_words(pairs_arg(f[1])))
    set_year(int(f[1]))
    if op == "N":
        return ";".join(",".join(hx(k) + ":" + val(v) for k, v in pairs) for pairs in cj.parse_ns_headers([unhex(x) for x in f[2:]]))
    if op == "H":
        return val(cj.http2time(unhex(f[2])))
    return val(cj.iso2time(unhex(f[2])))


def line(f):
    try:
        return answer(f)
    except (ValueError, IndexError) as e:
        return f"E:{type(e).__name__}:{e}"


out = [line(l.split(" ")[1:]) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
