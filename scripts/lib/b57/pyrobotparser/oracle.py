"""The oracle for pyrobotparser: CPython 3.13's urllib.robotparser, fed the
text as read() would (decoded, splitlines) and asked directly."""
import sys
import urllib.robotparser

assert sys.version_info[:2] == (3, 13), sys.version


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def hx(t):
    return t.encode("utf-8").hex() or "-"


def tryv(f):
    try:
        return f()
    except ValueError as e:
        return f"E:ValueError:{e}"


def line(f):
    p = urllib.robotparser.RobotFileParser()
    try:
        p.parse(unhex(f[0]).splitlines())
    except ValueError as e:
        return hx(f"E:ValueError:{e}")
    sm = p.site_maps()
    out = [str(p), "N" if sm is None else "L" + "\n".join(sm)]
    for k in range(1, len(f) - 1, 2):
        ua, url = unhex(f[k]), unhex(f[k + 1])
        out.append(tryv(lambda: "T" if p.can_fetch(ua, url) else "F"))
        out.append(repr(p.crawl_delay(ua)))
        rr = p.request_rate(ua)
        out.append("N" if rr is None else f"{rr.requests}/{rr.seconds}")
    return ",".join(hx(x) for x in out)


out = [line(l.split(" ")[1:]) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
