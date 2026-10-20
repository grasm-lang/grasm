"""The oracle for pymimetypes: a CPython 3.13 mimetypes.MimeTypes(), which
holds only Python's built-in tables (the module-level functions also read
the machine's mime.types files)."""
import mimetypes
import sys

assert sys.version_info[:2] == (3, 13), sys.version
M = mimetypes.MimeTypes()


def unhex(h):
    return "" if h == "-" else bytes.fromhex(h).decode("utf-8")


def opt(v):
    return "-" if v is None else "s" + v


def line(f):
    op, strict, arg = f[0], f[1] == "1", unhex(f[2])
    try:
        if op == "T":
            t, e = M.guess_type(arg, strict)
            return opt(t) + "|" + opt(e)
        if op == "F":
            t, e = M.guess_file_type(arg, strict=strict)
            return opt(t) + "|" + opt(e)
        if op == "A":
            return "L" + ",".join(M.guess_all_extensions(arg, strict))
        return opt(M.guess_extension(arg, strict))
    except ValueError as e:
        return f"E:ValueError:{e}"


out = [line(l.split(" ")[1:]).encode("utf-8").hex() or "-" for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
