"""The oracle for pystrptime: CPython 3.13's time.strptime and
datetime.strptime in the C locale with TZ=UTC. Answers are the hex of
their UTF-8."""
import os
import time
import warnings

os.environ["TZ"] = "UTC"
time.tzset()
import datetime  # noqa: E402
import sys  # noqa: E402

assert sys.version_info[:2] == (3, 13), sys.version
warnings.simplefilter("ignore")


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def answer(f):
    data, fmt = unhex(f[1]), unhex(f[2])
    if f[0] == "T":
        t = time.strptime(data, fmt)
        return ",".join(str(x) for x in tuple(t)) + "|" + ("N" if t.tm_zone is None else "s" + t.tm_zone) + "|" + ("N" if t.tm_gmtoff is None else str(t.tm_gmtoff))
    d = datetime.datetime.strptime(data, fmt)
    head = ",".join(str(x) for x in (d.year, d.month, d.day, d.hour, d.minute, d.second, d.microsecond))
    if d.tzinfo is None:
        return head + "|N|N"
    off = d.utcoffset()
    us = (off.days * 86400 + off.seconds) * 1000000 + off.microseconds
    return head + f"|{us}|s" + d.tzinfo.tzname(None)


def line(f):
    try:
        return answer(f)
    except Exception as e:  # noqa: BLE001 - the class is part of the answer
        return f"E:{type(e).__name__}:{e}"


out = [line(l.split(" ")[1:]).encode("utf-8").hex() or "-" for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
