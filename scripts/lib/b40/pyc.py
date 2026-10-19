"""Oracle for scripts/b40_diff.py: each .pyc's header and code objects as
the running Python reads them, printed as b40test prints its own reading.
Run under each Python whose files are checked."""
import marshal
import sys


def hs(s):
    return s.encode("utf-8", "surrogatepass").hex()


def code(c):
    names = "+".join(hs(n) for n in c.co_names)
    local = "+".join(hs(n) for n in c.co_varnames)
    nested = "".join(code(k) for k in c.co_consts if hasattr(k, "co_code"))
    qual = getattr(c, "co_qualname", c.co_name)
    return (f"({hs(c.co_name)},{hs(qual)},{hs(c.co_filename)},{c.co_firstlineno},{c.co_argcount},{c.co_posonlyargcount},"
            f"{c.co_kwonlyargcount},{c.co_flags},{c.co_stacksize},{names},{local},{nested})")


for f in sys.argv[1:]:
    d = open(f, "rb").read()
    magic = d[0] | d[1] << 8
    flags = int.from_bytes(d[4:8], "little")
    if flags & 1:
        head = f"{magic},{flags},0,0,{d[8:16].hex()}"
    else:
        head = f"{magic},{flags},{int.from_bytes(d[8:12], 'little')},{int.from_bytes(d[12:16], 'little')},"
    print(f + " " + head + "|" + code(marshal.loads(d[16:])))
