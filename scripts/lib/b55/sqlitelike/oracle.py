# The oracle for packages/sqlitelike: SQLite's like() and glob(), through
# CPython 3.13's sqlite3 module (SQLite 3.51.1). Values are encoded as for
# sqliteprintf; the answer is 1, 0, null, or E: and the error's message.
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
    fn = "glob" if f[0] == "g" else "like"
    try:
        r = db.execute(f"select {fn}({', '.join('?' * len(args))})", args).fetchone()[0]
        out.append("null" if r is None else str(r))
    except sqlite3.Error as e:
        out.append("E:" + str(e))
sys.stdout.write("".join(o + "\n" for o in out))
