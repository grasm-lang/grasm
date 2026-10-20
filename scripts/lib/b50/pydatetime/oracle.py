"""The oracle for packages/pydatetime: CPython 3.13's datetime (the C
module). One operation per line; one answer per line."""
import datetime as dt
import struct
import sys

SPECS = ["auto", "hours", "minutes", "seconds", "milliseconds", "microseconds"]


def bits(x):
    return "%x" % struct.unpack("<Q", struct.pack("<d", x))[0]


def text(h):
    return "" if h == "-" else "".join(chr(int(x, 16)) for x in h.split("."))


def hexs(s):
    return s.encode().hex() or "-"


def td_of(spec):
    d, s, u = (int(x) for x in spec.split(","))
    return dt.timedelta(days=d, seconds=s, microseconds=u)


def tz_of(spec):
    return None if spec == "~" else dt.timezone(td_of(spec))


def dt_of(spec):
    f, off = spec.split("/")
    return dt.datetime(*(int(x) for x in f.split(",")), tzinfo=tz_of(off))


def date_of(spec):
    return dt.date(*(int(x) for x in spec.split(",")))


def off(o):
    return "~" if o is None else f"{o.days}:{o.seconds}:{o.microseconds}"


def tdd(t):
    return f"T{t.days},{t.seconds},{t.microseconds}|{hexs(str(t))}|{hexs(repr(t))}|{bits(t.total_seconds())}"


def dated(d):
    y, w, k = d.isocalendar()
    return f"D{d.year},{d.month},{d.day}|{d.isoformat()}|{d.toordinal()}|{d.weekday()}|{d.isoweekday()}|{y},{w},{k}|{hexs(d.ctime())}"


def timed(t):
    return f"H{t.hour},{t.minute},{t.second},{t.microsecond},{off(t.utcoffset())}|" + "|".join(hexs(t.isoformat(s)) for s in SPECS)


def dtd(x):
    aware = x if x.tzinfo is not None else x.replace(tzinfo=dt.timezone.utc)
    name = "~" if x.tzinfo is None else hexs(str(x.tzinfo))
    return (f"X{x.year},{x.month},{x.day},{x.hour},{x.minute},{x.second},{x.microsecond},{off(x.utcoffset())}|{name}|{hexs(str(x))}|{hexs(x.ctime())}|"
            + "|".join(hexs(x.isoformat("T", s)) for s in SPECS) + f"|{bits(aware.timestamp())}")


def line(f):
    op = f[0]
    if op == "td":
        w, d, h, mi, s, ms, us = (int(x) for x in f[1:8])
        return tdd(dt.timedelta(weeks=w, days=d, hours=h, minutes=mi, seconds=s, milliseconds=ms, microseconds=us))
    if op == "t2":
        a, b = td_of(f[2]), td_of(f[3])
        k = f[1]
        if k == "add":
            return tdd(a + b)
        if k == "sub":
            return tdd(a - b)
        if k == "fdt":
            return "I" + str(a // b)
        if k == "mod":
            return tdd(a % b)
        return "F" + bits(a / b)
    if op == "t1":
        a, n = td_of(f[2]), int(f[3])
        k = f[1]
        r = a * n if k == "mul" else a // n if k == "fdi" else a / n if k == "tdi" else -a if k == "neg" else abs(a)
        return tdd(r)
    if op == "dn":
        return dated(dt.date(int(f[1]), int(f[2]), int(f[3])))
    if op == "do":
        return dated(dt.date.fromordinal(int(f[1])))
    if op == "dc":
        return dated(dt.date.fromisocalendar(int(f[1]), int(f[2]), int(f[3])))
    if op == "di":
        return dated(dt.date.fromisoformat(text(f[1])))
    if op == "da":
        d, t = date_of(f[1]), td_of(f[2])
        return dated(d + t if f[3] == "+" else d - t)
    if op == "dd":
        return tdd(date_of(f[1]) - date_of(f[2]))
    if op == "ti":
        return timed(dt.time.fromisoformat(text(f[1])))
    if op == "xi":
        return dtd(dt.datetime.fromisoformat(text(f[1])))
    if op == "xa":
        x, t = dt_of(f[1]), td_of(f[2])
        return dtd(x + t if f[3] == "+" else x - t)
    if op == "xd":
        return tdd(dt_of(f[1]) - dt_of(f[2]))
    if op == "xt":
        tz = dt.timezone(td_of(f[2]))
        return dtd(dt.datetime.fromtimestamp(struct.unpack("<d", struct.pack("<Q", int(f[1], 16)))[0], tz))
    if op == "tz":
        return hexs(str(dt.timezone(td_of(f[1]))))
    if op == "tn":
        return timed(dt.time(*(int(x) for x in f[1:5]), tzinfo=tz_of(f[5])))
    x = dt.datetime(*(int(v) for v in f[1:8]), tzinfo=tz_of(f[8]))
    return hexs(x.isoformat(chr(int(f[9])), f[10]))


def answer(f):
    try:
        return line(f)
    except (ValueError, OverflowError, ZeroDivisionError, TypeError) as e:
        return "E" + type(e).__name__ + ":" + hexs(str(e))


out = [answer(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
