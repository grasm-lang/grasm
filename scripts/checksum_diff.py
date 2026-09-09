"""Checksum files against what `shasum` and `md5` actually write."""
import random
import tempfile
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/cstest/src/main.gr")

random.seed(191)
work = tempfile.mkdtemp()
names = ["plain.txt", "with space.txt", "star*name.txt", "dash-name",
         "sub dir/file.bin", "unicode-café.txt"]
for name in names:
    path = os.path.join(work, name)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "wb").write(os.urandom(random.randint(0, 200)))

docs = []
expected = []
for algorithm, bits in (("1", 40), ("256", 64), ("512", 128)):
    for style in ("gnu", "bsd", "binary"):
        args = ["shasum", "-a", algorithm]
        if style == "bsd":
            args.append("--tag")
        if style == "binary":
            args.append("-b")
        ran = subprocess.run(args + names, cwd=work, capture_output=True,
                             text=True)
        if ran.returncode != 0:
            continue
        docs.append(ran.stdout)
        want = ""
        for line in ran.stdout.split("\n"):
            if not line.strip():
                continue
            if line.startswith("SHA"):
                head, _, digest = line.partition(") = ")
                algo, _, name = head.partition(" (")
                want += f"{digest}\x02{name}\x02{algo.upper()}\x02false\x01"
            else:
                digest, rest = line.split(" ", 1)
                binary = rest.startswith("*")
                name = rest[1:]
                algo = {40: "SHA1", 64: "SHA256", 128: "SHA512"}[len(digest)]
                want += (f"{digest}\x02{name}\x02{algo}\x02"
                         f"{'true' if binary else 'false'}\x01")
        expected.append(want)

got = subprocess.run([GRASM, "run", HARNESS, "--"] +
                     [d.encode("utf-8").hex() for d in docs],
                     capture_output=True, text=True)
if got.returncode != 0:
    print(got.stderr[-2000:]); sys.exit(1)
lines = got.stdout.split("\n")
agree = differ = shown = 0
for i, want in enumerate(expected):
    mine = bytes.fromhex(lines[i]).decode("utf-8") if lines[i] else ""
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print("  doc:", repr(docs[i][:200]))
            print("    shasum", repr(want[:250]))
            print("    grasm ", repr(mine[:250]))
print(f"files: {len(expected)}   agreeing: {agree}   differing: {differ}")

sys.exit(1 if differ else 0)
