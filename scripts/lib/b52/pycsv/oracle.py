"""The oracle for packages/pycsv: CPython 3.13's csv module (_csv). Text is
hex UTF-8, '-' when empty; a dialect is joined by '/'."""
import csv
import io
import struct
import sys


def text(h):
    return "" if h == "-" else bytes.fromhex(h).decode()


def hx(s):
    return s.encode().hex() or "-"


def ch(x):
    return None if x == "~" else chr(int(x))


def dialect(spec):
    f = spec.split("/")
    return dict(delimiter=ch(f[0]), quotechar=ch(f[1]), escapechar=ch(f[2]), doublequote=f[3] == "1", skipinitialspace=f[4] == "1",
                lineterminator=text(f[5]), quoting=int(f[6]), strict=f[7] == "1")


def value(v):
    if v is None:
        return "n"
    if isinstance(v, float):
        return "f%x" % struct.unpack("<Q", struct.pack("<d", v))[0]
    return "s" + hx(v)


def field(t):
    if t == "n":
        return None
    if t[0] == "i":
        return int(t[1:])
    if t[0] == "d":
        return struct.unpack("<d", struct.pack("<Q", int(t[1:], 16)))[0]
    return text(t[1:])


def line(f):
    try:
        if f[0] == "rd":
            rows = list(csv.reader(io.StringIO(text(f[1]), newline=""), **dialect(f[2])))
            return "/".join(",".join(value(v) for v in r) or "~" for r in rows) or "-"
        if f[0] == "wr":
            out = io.StringIO()
            fields = [] if f[1] == "~" else [field(x) for x in f[1].split(",")]
            csv.writer(out, **dialect(f[2])).writerow(fields)
            return hx(out.getvalue())
        return "f%x" % struct.unpack("<Q", struct.pack("<d", float(text(f[1]))))[0]
    except csv.Error as e:
        return "Ecsv:" + hx(str(e))
    except ValueError as e:
        return "Evalue:" + hx(str(e))
    except TypeError as e:
        return "Etype:" + hx(str(e))


out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
