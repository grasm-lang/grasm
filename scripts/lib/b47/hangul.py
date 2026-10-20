"""The oracle for packages/hangul: Python 3.13's unicodedata. Case files
named on stdin; each line `op arg` answers one hex value, joined by `;`."""
import sys
import unicodedata as u

hx = lambda s: s.encode("utf-8", "surrogatepass").hex() or "-"
unhex = lambda h: "" if h == "-" else bytes.fromhex(h).decode("utf-8", "surrogatepass")


def answer(f):
    op = f[0]
    if op == "nfd":
        return u.normalize("NFD", unhex(f[1]))
    if op == "nfc":
        return u.normalize("NFC", unhex(f[1]))
    if op == "compat":
        return u.normalize("NFKD", unhex(f[1]))
    if op == "name":
        # The package names syllables; Python names everything.
        n = u.name(chr(int(f[1], 16)), "null")
        return n if n.startswith("HANGUL SYLLABLE ") else "null"
    if op == "lookup":
        try:
            return format(ord(u.lookup(unhex(f[1]))), "x")
        except KeyError:
            return "null"
    if op == "decomp":
        return " ".join(format(ord(c), "x") for c in u.normalize("NFD", chr(int(f[1], 16))))
    return "unknown"


for path in sys.stdin.read().split():
    print(";".join(hx(answer(l.split(" "))) for l in open(path, encoding="utf-8").read().splitlines() if l))
