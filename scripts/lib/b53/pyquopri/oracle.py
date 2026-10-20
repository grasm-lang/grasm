# The oracle for packages/pyquopri: CPython 3.13's binascii.b2a_qp and
# a2b_qp. Data is hex, '-' when empty.
import binascii
import sys


def unhex(h):
    return b"" if h == "-" else bytes.fromhex(h)


def line(f):
    if f[0] == "en":
        return binascii.b2a_qp(unhex(f[1]), quotetabs=f[2] == "1", istext=f[3] == "1", header=f[4] == "1").hex() or "-"
    return binascii.a2b_qp(unhex(f[1]), header=f[2] == "1").hex() or "-"


out = []
for l in sys.stdin:
    l = l.rstrip("\n")
    if l:
        out.append(line(l.split(" ")))
sys.stdout.write("".join(o + "\n" for o in out))
