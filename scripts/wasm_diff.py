"""Random wasm modules, for `scripts/wasm_diff.sh`.

Each module is one exported function of two `i32` parameters whose body is a
random expression tree over the instructions the interpreter implements. The
point is the instructions nobody picks by hand: a shift is not in this set, but
signed and unsigned division, remainder, and every comparison are, and those
are where a 64-bit host's habits leak into a 32-bit machine.
"""
import random, sys, os

def leb(n):
    out = bytearray()
    while True:
        b = n & 0x7f
        n >>= 7
        if n == 0:
            out.append(b)
            return bytes(out)
        out.append(b | 0x80)

def sleb(n):
    out = bytearray()
    while True:
        b = n & 0x7f
        n >>= 7
        if (n == 0 and not b & 0x40) or (n == -1 and b & 0x40):
            out.append(b)
            return bytes(out)
        out.append(b | 0x80)

def sec(i, body):
    return bytes([i]) + leb(len(body)) + body

def vec(items):
    return leb(len(items)) + b"".join(items)

# opcode, name — every one the interpreter claims to implement.
BINOPS = [
    (0x6a, "add"), (0x6b, "sub"), (0x6c, "mul"),
    (0x6d, "div_s"), (0x6e, "div_u"), (0x6f, "rem_s"), (0x70, "rem_u"),
    (0x71, "and"),
    (0x46, "eq"), (0x47, "ne"),
    (0x48, "lt_s"), (0x49, "lt_u"), (0x4a, "gt_s"), (0x4b, "gt_u"),
    (0x4c, "le_s"), (0x4d, "le_u"), (0x4e, "ge_s"), (0x4f, "ge_u"),
]
UNOPS = [(0x45, "eqz"), (0x67, "clz"), (0x68, "ctz"), (0x69, "popcnt")]

def expr(rng, depth):
    if depth <= 0 or rng.random() < 0.3:
        c = rng.choice([0, 1, -1, 2, 7, 255, 65536, 2147483647, -2147483648,
                        rng.randrange(-100, 100)])
        if rng.random() < 0.5:
            return b"\x20" + leb(rng.randrange(2))       # local.get
        return b"\x41" + sleb(c)                          # i32.const
    if rng.random() < 0.15:
        op, _ = rng.choice(UNOPS)
        return expr(rng, depth - 1) + bytes([op])
    op, _ = rng.choice(BINOPS)
    return expr(rng, depth - 1) + expr(rng, depth - 1) + bytes([op])

def module(rng):
    body = b"\x00" + expr(rng, 4) + b"\x0b"
    types = vec([b"\x60" + vec([b"\x7f", b"\x7f"]) + vec([b"\x7f"])])
    return (b"\x00asm\x01\x00\x00\x00"
            + sec(1, types) + sec(3, vec([leb(0)]))
            + sec(7, vec([leb(1) + b"f" + b"\x00" + leb(0)]))
            + sec(10, vec([leb(len(body)) + body])))

def main():
    rng = random.Random(int(sys.argv[1]))
    n = int(sys.argv[2])
    out = sys.argv[3]
    args = [(0, 0), (1, 1), (-1, 1), (2147483647, 1), (-2147483648, -1),
            (7, 3), (-7, 3), (7, -3), (0, 5), (5, 0), (255, 8), (1000, 7)]
    lines = []
    for _ in range(n):
        m = module(rng).hex()
        a, b = rng.choice(args)
        lines.append(f"{m} f {a} {b}")
    open(os.path.join(out, "cases.txt"), "w").write("\n".join(lines) + "\n")
    print(f"{len(lines)} modules written")

main()
