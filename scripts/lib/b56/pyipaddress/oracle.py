"""The oracle for pyipaddress: CPython 3.13's ipaddress module, asked
directly. Each answer is the hex of its UTF-8 text, since scope ids may
hold any character."""
import ipaddress as I
import itertools
import sys

assert sys.version_info[:2] == (3, 13), sys.version


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def err(e):
    return f"E:{type(e).__name__}:{e}"


def tryv(f):
    try:
        return str(f())
    except Exception as e:  # noqa: BLE001 - the class is part of the answer
        return err(e)


def b(v):
    return "T" if v else "F"


def opt(v):
    return "-" if v is None else str(v)


def addr_flags(a):
    fs = [a.is_multicast, a.is_private, a.is_global, a.is_unspecified, a.is_reserved, a.is_loopback, a.is_link_local]
    if a.version == 6:
        fs.append(a.is_site_local)
    return "".join(b(f) for f in fs)


def v6_extras(a):
    t = a.teredo
    return [opt(a.ipv4_mapped), "-" if t is None else f"{t[0]},{t[1]}", opt(a.sixtofour), "N" if a.scope_id is None else "s" + a.scope_id]


def addr_desc(a):
    out = [repr(a), str(a), tryv(lambda: a.exploded), a.compressed, tryv(lambda: a.reverse_pointer), a.packed.hex(), str(int(a)), str(a.version),
           str(a.max_prefixlen), addr_flags(a)]
    if a.version == 6:
        out += v6_extras(a)
    else:
        out.append(tryv(lambda: a.ipv6_mapped))
    return "|".join(out)


def iface_desc(a):
    out = [repr(a), str(a), tryv(lambda: a.exploded), a.compressed, tryv(lambda: a.reverse_pointer), a.packed.hex(), str(int(a)), repr(a.ip),
           repr(a.network), str(a.netmask), str(a.hostmask), a.with_prefixlen, a.with_netmask, a.with_hostmask, addr_flags(a)]
    if a.version == 6:
        out += v6_extras(a)
    else:
        out.append(tryv(lambda: a.ipv6_mapped))
    return "|".join(out)


def net_flags(n):
    fs = [n.is_multicast, n.is_private, n.is_global, n.is_unspecified, n.is_reserved, n.is_loopback, n.is_link_local]
    if n.version == 6:
        fs.append(n.is_site_local)
    return "".join(b(f) for f in fs)


def first(gen, k):
    got = list(itertools.islice(gen, k + 1))
    return ",".join(repr(x) for x in got[:k]) + ("+" if len(got) > k else "")


def net_desc(n):
    return "|".join([repr(n), str(n), tryv(lambda: n.exploded), n.compressed, tryv(lambda: n.reverse_pointer), repr(n.network_address),
                     str(n.broadcast_address), str(n.netmask), str(n.hostmask), str(n.prefixlen), n.with_prefixlen, n.with_netmask, n.with_hostmask,
                     str(n.num_addresses), net_flags(n), first(n.hosts(), 3), repr(n[0]), repr(n[-1])])


CTOR = {"A": {"ip": I.ip_address, "v4": I.IPv4Address, "v6": I.IPv6Address},
        "I": {"ip": I.ip_interface, "v4": I.IPv4Interface, "v6": I.IPv6Interface}}


def make(kind, ctor, text):
    if kind == "N":
        strict = ctor.endswith("s")
        c = {"ip": lambda t: I.ip_network(t, strict), "v4": lambda t: I.IPv4Network(t, strict), "v6": lambda t: I.IPv6Network(t, strict)}[ctor[:-1]]
        return c(text)
    return CTOR[kind][ctor](text)


def arg(spec):
    # kind:ctor:hex
    k, c, h = spec.split(":")
    return make(k, c, unhex(h))


def answer(f):
    op = f[0]
    if op in ("A", "I", "N"):
        x = make(op, f[1], unhex(f[2]))
        return {"A": addr_desc, "I": iface_desc, "N": net_desc}[op](x)
    if op == "AI":
        cls = I.IPv4Address if f[1] == "4" else I.IPv6Address
        return addr_desc(cls(int(f[2])))
    if op == "AP":
        cls = I.IPv4Address if f[1] == "4" else I.IPv6Address
        return addr_desc(cls(bytes.fromhex("" if f[2] == "-" else f[2])))
    if op == "ADD" or op == "SUB":
        x = arg(f[1])
        d = int(f[2])
        r = x + d if op == "ADD" else x - d
        return repr(r)
    if op == "CMP":
        x, y = arg(f[1]), arg(f[2])
        return "".join(tryv(lambda: b(g())) + ";" for g in [lambda: x == y, lambda: x != y, lambda: x < y, lambda: x <= y, lambda: x > y,
                                                                lambda: x >= y])
    if op == "GI":
        return repr(arg(f[1])[int(f[2])])
    if op == "SUBN":
        n = arg(f[1])
        new = None if f[3] == "-" else int(f[3])
        return first(n.subnets(int(f[2]), new), 4)
    if op == "SUPN":
        n = arg(f[1])
        new = None if f[3] == "-" else int(f[3])
        return repr(n.supernet(int(f[2]), new))
    if op == "EXC":
        return ",".join(repr(x) for x in arg(f[1]).address_exclude(arg(f[2])))
    if op == "NN":
        x, y = arg(f[1]), arg(f[2])
        return ";".join(tryv(g) for g in [lambda: b(x.overlaps(y)), lambda: b(x.subnet_of(y)), lambda: b(x.supernet_of(y)),
                                         lambda: x.compare_networks(y)])
    if op == "IN":
        return b(arg(f[2]) in arg(f[1]))
    if op == "SAR":
        return ",".join(repr(x) for x in I.summarize_address_range(arg(f[1]), arg(f[2])))
    if op == "COL":
        return ",".join(repr(x) for x in I.collapse_addresses([arg(s) for s in f[1:]]))
    if op == "FMT":
        return format(arg(f[1]), unhex(f[2]))
    raise SystemExit("unknown op " + op)


def line(f):
    try:
        out = answer(f)
    except Exception as e:  # noqa: BLE001
        out = err(e)
    return out.encode("utf-8", "surrogatepass").hex() or "-"


out = [line(l.split(" ")[1:]) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
