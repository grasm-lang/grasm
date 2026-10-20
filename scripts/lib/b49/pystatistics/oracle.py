"""The oracle for packages/pystatistics: Python 3.13's statistics, over
floats given as the hex of their bits and lists joined by `/`."""
import statistics as st
import struct
import sys


def f(h):
    return struct.unpack("<d", struct.pack("<Q", int(h, 16)))[0]


def fh(x):
    return "%x" % struct.unpack("<Q", struct.pack("<d", float(x)))[0]


def fl(t):
    return [f(x) for x in t.split("/")] if t else []


def opt(t):
    return None if t == "~" else f(t)


def run(op, a):
    if op in ("mean", "fmean", "geometric_mean", "median", "median_low", "median_high", "mode"):
        return fh(getattr(st, op)(fl(a[0])))
    if op == "fmeanw":
        return fh(st.fmean(fl(a[0]), fl(a[1])))
    if op == "harmonic":
        return fh(st.harmonic_mean(fl(a[0]), None if a[1] == "~" else fl(a[1])))
    if op == "grouped":
        return fh(st.median_grouped(fl(a[0]), f(a[1])))
    if op == "multimode":
        return "/".join(fh(x) for x in st.multimode(fl(a[0]))) or "-"
    if op == "quantiles":
        return "/".join(fh(x) for x in st.quantiles(fl(a[0]), n=int(a[1]), method=a[2])) or "-"
    if op in ("variance", "pvariance", "stdev", "pstdev"):
        return fh(getattr(st, op)(fl(a[0]), opt(a[1])))
    if op == "covariance":
        return fh(st.covariance(fl(a[0]), fl(a[1])))
    if op == "correlation":
        return fh(st.correlation(fl(a[0]), fl(a[1]), method=a[2]))
    if op == "regression":
        r = st.linear_regression(fl(a[0]), fl(a[1]), proportional=a[2] == "1")
        return fh(r.slope) + "/" + fh(r.intercept)
    if op == "samples":
        d = st.NormalDist.from_samples(fl(a[0]))
        return fh(d.mean) + "/" + fh(d.stdev)
    d = st.NormalDist(f(a[0]), f(a[1]))
    if op == "pdf":
        return fh(d.pdf(f(a[2])))
    if op == "zscore":
        return fh(d.zscore(f(a[2])))
    if op == "invcdf":
        return fh(d.inv_cdf(f(a[2])))
    return "/".join(fh(x) for x in d.quantiles(int(a[2]))) or "-"


out = []
for line in sys.stdin.read().splitlines():
    if not line:
        continue
    op, *a = line.split(" ")
    try:
        out.append(run(op, a))
    except st.StatisticsError as e:
        out.append("S" + str(e).encode().hex())
    except ValueError as e:
        out.append("V" + str(e).encode().hex())
    except AttributeError as e:
        out.append("A" + str(e).encode().hex())
print("\n".join(out))
