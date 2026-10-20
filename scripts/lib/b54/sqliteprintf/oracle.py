# The oracle for packages/sqliteprintf: SQLite's printf(), through CPython
# 3.13's sqlite3 module (SQLite 3.51.1). A value is 'z' (NULL), 'i<decimal>'
# (INTEGER), 'n<hex of the double's bits>' (REAL), 't<hex of UTF-8>' (TEXT)
# or 'b<hex>' (BLOB); the answer is 's:' and the result's bytes in hex, or
# 'null'.
import sqlite3
import struct
import sys

db = sqlite3.connect(":memory:")
db.text_factory = bytes


def value(v):
    if v == "z":
        return None
    if v[0] == "n":
        return struct.unpack(">d", bytes.fromhex(v[1:]))[0]
    if v[0] == "i":
        return int(v[1:])
    if v[0] == "b":
        return b"" if v == "b-" else bytes.fromhex(v[1:])
    return "" if v == "t-" else bytes.fromhex(v[1:]).decode("utf-8")


out = []
for l in sys.stdin:
    f = l.split()
    if not f:
        continue
    args = [value(v) for v in f[1:]]
    r = db.execute(f"select printf({', '.join('?' * len(args))})", args).fetchone()[0]
    out.append("null" if r is None else "s:" + (r.hex() or "-"))
sys.stdout.write("".join(o + "\n" for o in out))
