"""The oracle for pystr: CPython 3.13's str methods, called directly."""
import sys
import unicodedata

assert sys.version_info[:2] == (3, 13) and unicodedata.unidata_version == "15.1.0", sys.version


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def hx(t):
    return t.encode("utf-8").hex() or "-"


def opt_text(h):
    return None if h == "N" else unhex(h)


def opt_int(h):
    return None if h == "N" else int(h)


def lst(xs):
    return "L" + ",".join(hx(x) for x in xs)


def cps(t):
    return ".".join("%x" % ord(c) for c in t)


def cased(c):
    return c.islower() or c.isupper() or c.istitle()


def ignorable(c):
    if cased(c):
        return ("AΣ" + c).lower()[1] == "ς"
    return ("A" + c + "Σ").lower()[-1] == "ς"


def table(lo, hi):
    out = []
    for cp in range(lo, hi):
        if 0xD800 <= cp <= 0xDFFF:
            continue
        c = chr(cp)
        lw, up, ti = c.lower(), c.upper(), c.title()
        flags = (1 if cased(c) else 0) + (2 if ignorable(c) else 0)
        if lw != c or up != c or ti != c or flags:
            out.append(f"{cp:x}:{cps(lw)}/{cps(up)}/{cps(ti)}/{flags}")
    return ",".join(out)


def answer(f):
    op = f[0]
    if op == "rg":
        return table(int(f[1], 16), int(f[2], 16))
    s = unhex(f[1])
    if op in ("sp", "rsp"):
        return lst((s.split if op == "sp" else s.rsplit)(opt_text(f[2]), int(f[3])))
    if op == "sl":
        return lst(s.splitlines(f[2] == "1"))
    if op in ("strip", "lstrip", "rstrip"):
        return "S" + hx(getattr(s, op)(opt_text(f[2])))
    if op == "exp":
        return "S" + hx(s.expandtabs(int(f[2])))
    if op in ("part", "rpart"):
        return lst((s.partition if op == "part" else s.rpartition)(unhex(f[2])))
    if op in ("center", "ljust", "rjust"):
        return "S" + hx(getattr(s, op)(int(f[2]), unhex(f[3])))
    if op == "zfill":
        return "S" + hx(s.zfill(int(f[2])))
    if op in ("find", "rfind", "index", "rindex", "count"):
        return str(getattr(s, op)(unhex(f[2]), opt_int(f[3]), opt_int(f[4])))
    if op in ("startswith", "endswith"):
        return "T" if getattr(s, op)(unhex(f[2]), opt_int(f[3]), opt_int(f[4])) else "F"
    if op == "replace":
        return "S" + hx(s.replace(unhex(f[2]), unhex(f[3]), int(f[4])))
    if op in ("upper", "lower", "title", "capitalize", "swapcase"):
        return "S" + hx(getattr(s, op)())
    raise SystemExit("unknown op " + op)


def line(f):
    try:
        return answer(f)
    except (ValueError, TypeError) as e:
        return f"E:{type(e).__name__}:{e}"


out = [line(l.split(" ")[1:]) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
