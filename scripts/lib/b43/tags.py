"""The reference for b43_diff.py's tag family: pip's vendored packaging."""
import sys
from pip._vendor.packaging import tags

L = lambda s: [] if s == "_" else s.split(",")
V = lambda s: tuple(map(int, s.split(".")))
hx = lambda s: s.encode().hex()
out = []
for line in open(sys.argv[1]).read().splitlines():
    if not line:
        continue
    f = line.split(" ")
    if f[0] == "cp":
        out.append(hx(" ".join(str(t) for t in tags.cpython_tags(V(f[1]), L(f[2]), L(f[3])))))
    elif f[0] == "gen":
        out.append(hx(" ".join(str(t) for t in tags.generic_tags(f[1], L(f[2]), L(f[3])))))
    elif f[0] == "compat":
        out.append(hx(" ".join(str(t) for t in tags.compatible_tags(V(f[1]), None if f[2] == "_" else f[2], L(f[3])))))
    elif f[0] == "mac":
        out.append(hx(" ".join(tags.mac_platforms(V(f[1]), f[2]))))
    elif f[0] == "rank":
        supported = list(tags.cpython_tags(V(f[1]), L(f[2]), L(f[3]))) + list(tags.compatible_tags(V(f[1]), None, L(f[3])))
        wheel = set()
        for t in L(f[4]):
            wheel |= tags.parse_tag(bytes.fromhex(t).decode())
        out.append(next((str(i) for i, t in enumerate(supported) if t in wheel), "-"))
    else:
        try:
            out.append(hx(" ".join(sorted(str(t) for t in tags.parse_tag(bytes.fromhex(f[1]).decode())))))
        except ValueError:
            out.append("ERR")
print("\n".join(out))
