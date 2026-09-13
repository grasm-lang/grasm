"""Deflate, gzip and xz against zlib, gzip and lzma themselves."""
import gzip
import io
import lzma
import os
import pickle
import random
import struct
import subprocess
import sys
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))


def run(harness, args):
    ran = subprocess.run([GRASM, "run", os.path.join(ROOT, harness), "--"]
                         + args, capture_output=True, text=True)
    if ran.returncode != 0:
        print(ran.stderr[-2000:]); sys.exit(1)
    return ran.stdout.split("\n")


failures = 0


def note(label, detail):
    global failures
    failures += 1
    if failures <= 6:
        print(f"  {label}: {detail}")


# ---- deflate: everything it writes has to be readable by zlib itself.
random.seed(21)
payloads = [b"", b"a", b"ab", b"abc", b"aaaaaaaaaa", b"a" * 300,
            b"a" * 70000, b"hello hello hello hello",
            b"the quick brown fox " * 20, bytes(range(256)),
            bytes(range(256)) * 4]
for n in (1, 2, 3, 4, 5, 17, 64, 255, 256, 257, 1000, 5000):
    payloads.append(bytes(random.randrange(256) for _ in range(n)))
for n in (100, 1000, 20000):
    payloads.append(bytes(random.choice(b"abcdefg ") for _ in range(n)))
payloads.append(b"".join(bytes([random.randrange(3)])
                         * random.randrange(1, 300) for _ in range(200)))

args = []
for p in payloads:
    args += ["deflate", ":" + p.hex(), "roundtrip", ":" + p.hex(),
             "zlib", ":" + p.hex()]
lines = run("packages/dftest/src/main.gr", args)
smaller = 0
for i, p in enumerate(payloads):
    raw = bytes.fromhex(lines[i * 3]) if lines[i * 3] else b""
    back = bytes.fromhex(lines[i * 3 + 1]) if lines[i * 3 + 1] else b""
    wrapped = bytes.fromhex(lines[i * 3 + 2]) if lines[i * 3 + 2] else b""
    try:
        if zlib.decompress(raw, -15) != p:
            note("deflate", f"{len(p)} bytes: zlib decoded it differently")
    except Exception as e:
        note("deflate", f"{len(p)} bytes: zlib refused it ({e})")
    try:
        if zlib.decompress(wrapped) != p:
            note("zlib wrapper", f"{len(p)} bytes: decoded differently")
    except Exception as e:
        note("zlib wrapper", f"{len(p)} bytes: refused ({e})")
    if back != p:
        note("deflate", f"{len(p)} bytes: our own inflate disagrees")
    if len(raw) < len(p):
        smaller += 1
deflate_cases = len(payloads) * 3

# ---- gzip: both directions.
random.seed(31)
gz_cases = []


def described(members):
    return " ".join("%s|%s|%d|%s" % (d.hex(), n, m, c)
                    for d, n, m, c in members)


for p in (b"", b"a", b"hello world\n", b"the quick brown fox " * 30,
          bytes(random.randrange(256) for _ in range(1000)), b"a" * 70000):
    for level in (0, 1, 6, 9):
        buf = io.BytesIO()
        with gzip.GzipFile(fileobj=buf, mode="wb", compresslevel=level,
                           filename="", mtime=0) as f:
            f.write(p)
        gz_cases.append((f"level {level}", buf.getvalue(),
                         described([(p, "", 0, "")])))
    buf = io.BytesIO()
    with gzip.GzipFile("data.txt", fileobj=buf, mode="wb",
                       mtime=1700000000) as f:
        f.write(p)
    gz_cases.append(("named", buf.getvalue(),
                     described([(p, "data.txt", 1700000000, "")])))

first, second = io.BytesIO(), io.BytesIO()
with gzip.GzipFile(fileobj=first, mode="wb", mtime=0) as f:
    f.write(b"first\n")
with gzip.GzipFile(fileobj=second, mode="wb", mtime=0) as f:
    f.write(b"second\n")
gz_cases.append(("two members", first.getvalue() + second.getvalue(),
                 described([(b"first\n", "", 0, ""),
                            (b"second\n", "", 0, "")])))

# A header carrying every optional field, which `gzip` itself never writes.
body = zlib.compressobj(6, zlib.DEFLATED, -15)
compressed = body.compress(b"payload") + body.flush()
head = bytes([31, 139, 8, 0x04 | 0x08 | 0x10 | 0x02, 0, 0, 0, 0, 0, 255])
head += struct.pack("<H", 4) + b"AB\x02\x00"
head += b"name.bin\x00" + b"a comment\x00"
whole = (head + struct.pack("<H", zlib.crc32(head) & 0xffff) + compressed
         + struct.pack("<II", zlib.crc32(b"payload"), 7))
gz_cases.append(("every header field", whole,
                 described([(b"payload", "name.bin", 0, "a comment")])))

good = io.BytesIO()
with gzip.GzipFile(fileobj=good, mode="wb", mtime=0) as f:
    f.write(b"hello")
raw = bytearray(good.getvalue())
for position, label in ((-5, "wrong crc"), (-1, "wrong length")):
    broken = bytearray(raw); broken[position] ^= 0xff
    gz_cases.append((label, bytes(broken), "error"))
gz_cases.append(("truncated", bytes(raw[:-4]), "error"))
gz_cases.append(("not gzip", b"hello there", "error"))
gz_cases.append(("wrong method", bytes([31, 139, 9]) + bytes(raw[3:]),
                 "error"))

writes = [(b"", ""), (b"hello", ""), (b"hello", "greeting.txt"),
          (b"x" * 50000, "big.bin"),
          (bytes(random.randrange(256) for _ in range(3000)), "noise")]
args = []
for _, data, _ in gz_cases:
    args += ["read", ":" + data.hex()]
for data, name in writes:
    args += ["write", ":" + name.encode().hex(), ":" + data.hex()]
lines = run("packages/gztest/src/main.gr", args)
for i, (label, _, want) in enumerate(gz_cases):
    if lines[i] != want:
        note("gzip read", f"{label}: want {want[:60]} got {lines[i][:60]}")
for j, (data, name) in enumerate(writes):
    try:
        with gzip.GzipFile(fileobj=io.BytesIO(
                bytes.fromhex(lines[len(gz_cases) + j]))) as f:
            if f.read() != data:
                note("gzip write", f"{name}: gzip read it back differently")
    except Exception as e:
        note("gzip write", f"{name}: gzip refused it ({e})")

# ---- xz: every preset, every check kind, and each part corrupted in turn.
random.seed(41)
xz_cases = []
for p in (b"", b"a", b"hello world\n", b"the quick brown fox " * 40,
          bytes(range(256)),
          bytes(random.randrange(256) for _ in range(600)),
          b"".join(bytes([random.randrange(4)]) * random.randrange(1, 60)
                   for _ in range(120)),
          ("def f(x):\n    return x + 1\n" * 60).encode()):
    for preset in (0, 1, 6, 9):
        xz_cases.append((f"preset {preset}", lzma.compress(p, preset=preset),
                         p))
    for check in (lzma.CHECK_NONE, lzma.CHECK_CRC32, lzma.CHECK_CRC64,
                  lzma.CHECK_SHA256):
        xz_cases.append((f"check {check}", lzma.compress(p, check=check), p))
good = lzma.compress(b"hello world hello world")
for position, label in ((-6, "corrupt index"), (-2, "corrupt footer magic"),
                        (-12, "corrupt footer crc"),
                        (7, "corrupt header flags")):
    broken = bytearray(good); broken[position] ^= 0xff
    xz_cases.append((label, bytes(broken), None))
xz_cases.append(("not xz", b"hello there this is not xz at all", None))
xz_cases.append(("truncated", good[:20], None))
xz_cases.append(("two streams",
                 lzma.compress(b"first ") + lzma.compress(b"second"),
                 b"first second"))
args = []
for _, data, _ in xz_cases:
    args += ["decode", ":" + data.hex()]
lines = run("packages/xztest/src/main.gr", args)
for i, (label, _, want) in enumerate(xz_cases):
    expect = "error" if want is None else want.hex()
    if lines[i] != expect:
        note("xz", f"{label}: want {expect[:50]} got {lines[i][:50]}")

total = deflate_cases + len(gz_cases) + len(writes) + len(xz_cases)
print(f"cases: {total}   failing: {failures}   "
      f"deflate made {smaller} of {len(payloads)} inputs smaller")
sys.exit(1 if failures else 0)
