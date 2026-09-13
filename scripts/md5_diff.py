"""MD5, HMAC-MD5 and name-based UUIDs against `hashlib` and `uuid`."""
import hashlib
import hmac
import os
import random
import subprocess
import sys
import uuid

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/md5test/src/main.gr")

random.seed(7)
cases = []
# RFC 1321 appendix A.5, which is the suite the standard itself ships.
for message in (b"", b"a", b"abc", b"message digest",
                b"abcdefghijklmnopqrstuvwxyz",
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
                b"0123456789", b"1234567890" * 8):
    cases.append(("md5", message))
# Every length around a block boundary, because padding is where MD5
# implementations go wrong and 56 is the length that has no room for it.
for n in list(range(0, 130)) + [191, 192, 193, 255, 256, 257, 1000]:
    cases.append(("md5", bytes(random.randrange(256) for _ in range(n))))
for n in (0, 1, 15, 16, 17, 63, 64, 65, 100):
    cases.append(("hmac", bytes(random.randrange(256) for _ in range(n)),
                  bytes(random.randrange(256)
                        for _ in range(random.randrange(200)))))
NAMESPACES = {"dns": uuid.NAMESPACE_DNS, "url": uuid.NAMESPACE_URL,
              "oid": uuid.NAMESPACE_OID, "x500": uuid.NAMESPACE_X500}
for space in NAMESPACES:
    for name in ("", "www.example.com", "python.org", "a", "été", "1.3.6.1"):
        cases.append(("v5", space, name.encode()))
        cases.append(("v3", space, name.encode()))

# Arguments carry a leading colon: an empty one does not survive the shell,
# and the empty message is the first vector RFC 1321 lists.
args = []
expected = []
for case in cases:
    if case[0] == "md5":
        args += ["md5", ":" + case[1].hex()]
        expected.append(hashlib.md5(case[1]).hexdigest())
    elif case[0] == "hmac":
        args += ["hmac", ":" + case[1].hex(), ":" + case[2].hex()]
        expected.append(hmac.new(case[1], case[2], hashlib.md5).hexdigest())
    else:
        args += [case[0], case[1], ":" + case[2].hex()]
        make = uuid.uuid5 if case[0] == "v5" else uuid.uuid3
        expected.append(str(make(NAMESPACES[case[1]], case[2].decode())))

ran = subprocess.run([GRASM, "run", HARNESS, "--"] + args,
                     capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-2000:]); sys.exit(1)
lines = ran.stdout.split("\n")
agree = differ = shown = 0
for i, want in enumerate(expected):
    mine = lines[i] if i < len(lines) else "<missing>"
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print("  case:", cases[i])
            print("    python", want)
            print("    grasm ", mine)
print(f"vectors: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
