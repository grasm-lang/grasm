"""Three text encodings against the standard library's own."""
import base64, binascii, os, quopri, random, subprocess, sys
random.seed(409)
blobs = [b"", b"a", b"hello", b"caf\xc3\xa9", bytes(range(256)),
         b"line one\r\nline two\r\n", b"trailing space \r\nand tab\t\r\n",
         b"=equals= and =3D", b"x" * 200, b"\x00\x00\x00\x00",
         b"\x00\x00\x00\x00abcd", b"short", b"ab", b"abc", b"abcd", b"abcde"]
for _ in range(200):
    blobs.append(bytes(random.randint(0, 255)
                       for _ in range(random.randint(0, 120))))
texts = [b"hello", b"caf=C3=A9", b"soft=\r\nbreak", b"=", b"=zz", b"a=20",
         b"line=\nnext"]
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
grasm = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
harness = os.path.join(ROOT, "packages/enctest3/src/main.gr")

def run(requests):
    out = []
    step = 120
    for at in range(0, len(requests), step):
        flat = []
        for one in requests[at:at + step]:
            flat += one
        got = subprocess.run([grasm, "run", harness, "--"] + flat,
                             capture_output=True, text=True)
        if got.returncode != 0:
            print(got.stderr[-2000:]); sys.exit(1)
        out += got.stdout.split("\n")[:len(requests[at:at + step])]
    return out

agree = differ = shown = 0
def judge(name, got, want):
    global agree, differ, shown
    if got == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 8:
            pw = want.encode() if isinstance(want, str) else want
            pg = got.encode() if isinstance(got, str) else got
            print(f"  {name}\n    python {pw.hex()}\n    grasm  {pg.hex()}")

qp = run([["qp", b.hex()] for b in blobs])
for i, blob in enumerate(blobs):
    want = quopri.encodestring(blob).decode()
    judge(f"qp len={len(blob)} {blob.hex()}", bytes.fromhex(qp[i]).decode(), want)

qpback = run([["qpback", t.hex()] for t in texts])
for i, text in enumerate(texts):
    judge(f"qp decode {text!r}", bytes.fromhex(qpback[i]),
          quopri.decodestring(text))

header = run([["qpheader", b.hex()] for b in blobs if b])
at = 0
for blob in blobs:
    if not blob:
        continue
    want = quopri.encodestring(blob, header=True, quotetabs=True).decode()
    at += 1

a85 = run([["a85", b.hex()] for b in blobs])
for i, blob in enumerate(blobs):
    judge(f"a85 {blob[:24]!r}", bytes.fromhex(a85[i]),
          base64.a85encode(blob))

b85 = run([["b85", b.hex()] for b in blobs])
for i, blob in enumerate(blobs):
    judge(f"b85 {blob[:24]!r}", bytes.fromhex(b85[i]),
          base64.b85encode(blob))

a85back = run([["a85back", base64.a85encode(b).hex()] for b in blobs])
for i, blob in enumerate(blobs):
    judge(f"a85 decode {blob[:24]!r}", bytes.fromhex(a85back[i]), blob)

short = [b for b in blobs if len(b) <= 45]
uu = run([["uu", b.hex()] for b in short])
for i, blob in enumerate(short):
    want = binascii.b2a_uu(blob).decode().rstrip("\n")
    judge(f"uu {blob[:24]!r}", bytes.fromhex(uu[i]).decode(), want)

uuback = run([["uuback", binascii.b2a_uu(b).decode().rstrip("\n").encode().hex()]
              for b in short])
for i, blob in enumerate(short):
    judge(f"uu decode {blob[:24]!r}", bytes.fromhex(uuback[i]), blob)

print(f"checks: {agree + differ}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
