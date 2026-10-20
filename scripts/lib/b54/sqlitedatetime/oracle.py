# The oracle for packages/sqlitedatetime: SQLite's date and time functions,
# called through CPython 3.13's sqlite3 module (SQLite 3.51.1). A value is
# 't<hex of UTF-8>' (TEXT), 'n<hex of the double's bits>' (REAL),
# 'i<decimal>' (INTEGER), 'b<hex>' (BLOB) or 'z' (NULL).
import sqlite3
import struct
import sys

db = sqlite3.connect(":memory:")
FUNCS = {"jd": "julianday", "ue": "unixepoch", "date": "date", "time": "time", "dt": "datetime", "sf": "strftime", "td": "timediff"}


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


def show(r):
    if r is None:
        return "null"
    if isinstance(r, float):
        return "r:" + struct.pack(">d", r).hex()
    if isinstance(r, int):
        return "i:" + str(r)
    return "s:" + (r.encode("utf-8").hex() or "-")


out = []
for l in sys.stdin:
    f = l.split()
    if not f:
        continue
    args = [value(v) for v in f[1:]]
    r = db.execute(f"select {FUNCS[f[0]]}({', '.join('?' * len(args))})", args).fetchone()[0]
    out.append(show(r))
sys.stdout.write("".join(o + "\n" for o in out))
