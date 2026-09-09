"""netrc files against Python's own `netrc` module."""
import netrc as pynetrc
import random
import tempfile
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/nrtest/src/main.gr")

random.seed(241)
HOSTS = ["example.test", "a.test", "b.test"]
LOGINS = ["alice", "bob", '"quoted name"', "with\\ space"]
PASSWORDS = ["s3cret", '"has space"', "p@ss", '""']
def document():
    lines = []
    for host in random.sample(HOSTS, random.randint(1, 3)):
        if random.random() < 0.5:
            lines.append(f"machine {host} login {random.choice(LOGINS)} "
                         f"password {random.choice(PASSWORDS)}")
        else:
            lines.append(f"machine {host}")
            lines.append(f"  login {random.choice(LOGINS)}")
            if random.random() < 0.4:
                lines.append(f"  account acct{random.randint(1, 9)}")
            lines.append(f"  password {random.choice(PASSWORDS)}")
    if random.random() < 0.4:
        lines.append("default login anon password anon@")
    if random.random() < 0.2:
        lines.append("macdef init")
        lines.append("machine evil login mallory")
        lines.append("")
    return "\n".join(lines) + "\n"

docs = [document() for _ in range(150)]
got = subprocess.run([GRASM, "run", HARNESS, "--"] +
                     [d.encode().hex() for d in docs],
                     capture_output=True, text=True)
if got.returncode != 0:
    print(got.stderr[-2000:]); sys.exit(1)
lines = got.stdout.split("\n")
work = tempfile.mkdtemp()
path = os.path.join(work, ".netrc")
agree = differ = shown = skipped = 0
for i, doc in enumerate(docs):
    open(path, "w").write(doc)
    os.chmod(path, 0o600)
    try:
        parsed = pynetrc.netrc(path)
    except Exception:
        skipped += 1
        continue
    want = ""
    for host, (login, account, password) in parsed.hosts.items():
        want += (host + "\x02" + (login or "") + "\x02" +
                 (account or "") + "\x02" + (password or "") + "\x01")
    mine = bytes.fromhex(lines[i]).decode("utf-8") if lines[i] else ""
    if sorted(mine.split("\x01")) == sorted(want.split("\x01")):
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 5:
            print("  doc:", repr(doc[:200]))
            print("    python", repr(want[:250]))
            print("    grasm ", repr(mine[:250]))
print(f"files: {len(docs)}   agreeing: {agree}   differing: {differ}   "
      f"python refused: {skipped}")
sys.exit(1 if differ else 0)
