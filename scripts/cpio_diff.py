"""cpio archives against `cpio(1)` itself, both directions.

Reading: archives written by `cpio -o` are parsed here and the entries
compared with what the files on disk actually are.
Writing: what `render` produces is handed back to `cpio -i`, and the
extracted tree is compared with the original.
"""
import os
import subprocess
import sys

import shutil
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/cpiotest/src/main.gr")

if not shutil.which("cpio"):
    print("cpio(1) is needed")
    sys.exit(1)
work = tempfile.mkdtemp()
tree = os.path.join(work, "tree")
os.makedirs(os.path.join(tree, "deep", "deeper"))
FILES = {
    "small.txt": b"hello\n",
    "empty.txt": b"",
    "binary.bin": bytes(range(256)) * 5,
    "deep/one.txt": b"one" * 100,
    "deep/deeper/two.txt": b"two" * 1000,
    "a-name-that-is-quite-long-for-a-file-and-pads-oddly.txt": b"x" * 7,
}
for name, body in FILES.items():
    path = os.path.join(tree, name)
    with open(path, "wb") as handle:
        handle.write(body)
os.symlink("small.txt", os.path.join(tree, "link.txt"))
names = sorted(FILES) + ["link.txt", "deep", "deep/deeper", "."]
agree = differ = 0
for fmt, flag in (("070701", "newc"), ("070707", "odc")):
    archive = os.path.join(work, f"archive-{flag}.cpio")
    listing = "\n".join(sorted(names))
    with open(archive, "wb") as handle:
        made = subprocess.run(["cpio", "-o", "-H", flag],
                              input=listing.encode(), cwd=tree,
                              stdout=handle, stderr=subprocess.DEVNULL)
    if made.returncode != 0:
        print(f"  cpio -o -H {flag} failed"); differ += 1; continue
    got = subprocess.run([GRASM, "run", HARNESS, "--", "list", archive],
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-1500:]); sys.exit(1)
    lines = got.stdout.strip("\n").split("\n")
    if lines[0] != fmt:
        print(f"  format: cpio wrote {flag}, grasm read {lines[0]!r}")
        differ += 1
        continue
    seen = {}
    for line in lines[1:]:
        parts = line.split("\x02")
        seen[parts[0]] = parts
    for name in names:
        path = os.path.join(tree, name)
        info = os.lstat(path)
        if name not in seen:
            print(f"  {flag}: {name} missing from the parse"); differ += 1
            continue
        parts = seen[name]
        kind = ("directory" if os.path.isdir(path) and
                not os.path.islink(path) else
                "symlink" if os.path.islink(path) else "file")
        want = [name, kind, str(info.st_mode & 0o7777),
                str(info.st_size if kind != "directory" else 0),
                str(int(info.st_mtime)), str(info.st_uid), str(info.st_gid)]
        mine = parts[:7]
        # The old format keeps a 6-digit octal uid and 11-digit mtime;
        # both fit here, so the fields are compared for every format.
        if mine == want:
            agree += 1
        else:
            differ += 1
            print(f"  {flag} {name}:\n    disk  {want}\n    grasm {mine}")
    # And the other direction: write it back and let `cpio` extract it.
    rewritten = os.path.join(work, f"rewritten-{flag}.cpio")
    subprocess.run([GRASM, "run", HARNESS, "--", "rewrite", archive,
                    rewritten, fmt], capture_output=True, text=True)
    out = os.path.join(work, f"out-{flag}")
    os.makedirs(out, exist_ok=True)
    with open(rewritten, "rb") as handle:
        back = subprocess.run(["cpio", "-i", "-d", "--no-preserve-owner"],
                              stdin=handle, cwd=out,
                              capture_output=True)
    if back.returncode != 0:
        print(f"  cpio could not read what render wrote ({flag}): "
              f"{back.stderr.decode()[:200]}")
        differ += 1
        continue
    for name, body in FILES.items():
        path = os.path.join(out, name)
        if not os.path.exists(path) or open(path, "rb").read() != body:
            print(f"  {flag}: {name} did not come back the same")
            differ += 1
        else:
            agree += 1
    link = os.path.join(out, "link.txt")
    if os.path.islink(link) and os.readlink(link) == "small.txt":
        agree += 1
    else:
        print(f"  {flag}: the symbolic link did not come back")
        differ += 1
shutil.rmtree(work, ignore_errors=True)
print(f"checks: {agree + differ}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
