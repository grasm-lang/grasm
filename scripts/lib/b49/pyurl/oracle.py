"""The oracle for packages/pyurlparse: Python 3.13's urllib.parse. Each
input line is an operation with hex arguments; each answer is one line."""
import sys
import urllib.parse as up


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def hx(s):
    if s is None:
        return "~"
    if isinstance(s, bytes):
        return s.hex() or "-"
    return s.encode("utf-8", "surrogatepass").hex() or "-"


def split_desc(r):
    try:
        port = str(r.port) if r.port is not None else "~"
    except ValueError as e:
        port = "E" + hx(str(e))
    return ",".join([hx(x) for x in r] + [hx(r.username), hx(r.password), hx(r.hostname), port, hx(r.geturl())])


def run(f):
    op = f[0]
    if op == "us":
        return split_desc(up.urlsplit(unhex(f[1]), unhex(f[2]), f[3] == "1"))
    if op == "up":
        return split_desc(up.urlparse(unhex(f[1]), unhex(f[2]), f[3] == "1"))
    if op == "uj":
        return hx(up.urljoin(unhex(f[1]), unhex(f[2]), f[3] == "1"))
    if op == "ud":
        r = up.urldefrag(unhex(f[1]))
        return hx(r.url) + " " + hx(r.fragment)
    if op == "q":
        return hx(up.quote(unhex(f[1]), unhex(f[2])))
    if op == "qp":
        return hx(up.quote_plus(unhex(f[1]), unhex(f[2])))
    if op == "qb":
        return hx(up.quote_from_bytes(bytes.fromhex(f[1]) if f[1] != "-" else b"", unhex(f[2])))
    if op == "uq":
        return hx(up.unquote(unhex(f[1])))
    if op == "uqp":
        return hx(up.unquote_plus(unhex(f[1])))
    if op == "uqb":
        return hx(up.unquote_to_bytes(unhex(f[1])))
    if op == "qsl":
        mx = int(f[4])
        pairs = up.parse_qsl(unhex(f[1]), f[2] == "1", f[3] == "1", max_num_fields=None if mx < 0 else mx, separator=unhex(f[5]))
        return ",".join(hx(k) + "=" + hx(v) for k, v in pairs) or "-"
    if op == "qs":
        d = up.parse_qs(unhex(f[1]), f[2] == "1")
        return ",".join(hx(k) + "=" + "+".join(hx(v) for v in vs) for k, vs in d.items()) or "-"
    pairs = []
    for item in f[1:]:
        k, _, vs = item.partition("=")
        pairs.append((unhex(k), [unhex(v) for v in vs.split("+")]))
    return hx(up.urlencode(pairs, doseq=True))


out = []
for line in sys.stdin.read().splitlines():
    if not line:
        continue
    try:
        out.append(run(line.split(" ")))
    except ValueError as e:
        out.append("E " + hx(str(e)))
print("\n".join(out))
