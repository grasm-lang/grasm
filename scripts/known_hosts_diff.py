"""known_hosts matching against OpenSSH's own `ssh-keygen -F`."""
import os
import subprocess
import sys

import random
import re
import shutil
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/khtest/src/main.gr")

random.seed(251)
KEYS = tempfile.mkdtemp()

def key(name, kind="ed25519"):
    """Make a host key, because the file needs real ones to be read."""
    path = os.path.join(KEYS, name)
    if not os.path.exists(path + ".pub"):
        made = subprocess.run(["ssh-keygen", "-q", "-t", kind, "-N", "",
                               "-f", path, "-C", ""],
                              capture_output=True, text=True)
        if made.returncode != 0:
            print("ssh-keygen is needed")
            sys.exit(1)
    return open(path + ".pub").read().strip()
PLAIN = [key("k1"), key("k2"), key("k3"),
         key("r1", "rsa"), key("r2", "rsa")]
NAMES = ["example.test", "a.example.test", "b.example.test", "host1",
         "host2", "[example.test]:2222", "10.0.0.1"]
PATTERNS = ["example.test", "*.example.test", "host?",
            "*.example.test,!b.example.test", "*", "10.0.0.*",
            "a.example.test,10.0.0.1", "!host1,host*"]
MARKERS = ["", "", "", "@revoked ", "@cert-authority "]
def document():
    lines = ["# a comment", ""]
    for pattern in random.sample(PATTERNS, random.randint(1, 4)):
        lines.append(random.choice(MARKERS) + pattern + " " +
                     random.choice(PLAIN))
    return "\n".join(lines) + "\n"

work = tempfile.mkdtemp()
path = os.path.join(work, "known_hosts")
agree = differ = shown = 0
total = 0
for _ in range(60):
    doc = document()
    open(path, "w").write(doc)
    hosts = NAMES + ["c.example.test", "nothere"]
    got = subprocess.run([GRASM, "run", HARNESS, "--",
                          doc.encode().hex()] +
                         [h.encode().hex() for h in hosts],
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-2000:]); sys.exit(1)
    mine = got.stdout.split("\n")
    for i, host in enumerate(hosts):
        out = subprocess.run(["ssh-keygen", "-F", host, "-f", path],
                             capture_output=True, text=True)
        want = ",".join(re.findall(r"found: line (\d+)", out.stdout))
        total += 1
        if mine[i] == want:
            agree += 1
        else:
            differ += 1; shown += 1
            if shown <= 6:
                print(f"  host {host!r} in\n{doc}"
                      f"    ssh-keygen {want!r}\n    grasm      {mine[i]!r}")
    # And the same file after OpenSSH hashes every name in it.
    subprocess.run(["ssh-keygen", "-H", "-f", path],
                   capture_output=True, text=True)
    hashed = open(path).read()
    got = subprocess.run([GRASM, "run", HARNESS, "--",
                          hashed.encode().hex()] +
                         [h.encode().hex() for h in hosts],
                         capture_output=True, text=True)
    mine = got.stdout.split("\n")
    for i, host in enumerate(hosts):
        out = subprocess.run(["ssh-keygen", "-F", host, "-f", path],
                             capture_output=True, text=True)
        want = ",".join(re.findall(r"found: line (\d+)", out.stdout))
        total += 1
        if mine[i] == want:
            agree += 1
        else:
            differ += 1; shown += 1
            if shown <= 6:
                print(f"  hashed host {host!r}\n{hashed}"
                      f"    ssh-keygen {want!r}\n    grasm      {mine[i]!r}")
    for stale in (path + ".old",):
        if os.path.exists(stale):
            os.remove(stale)
shutil.rmtree(KEYS, ignore_errors=True)
print(f"lookups: {total}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
