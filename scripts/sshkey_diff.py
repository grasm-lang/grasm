"""OpenSSH public keys against what `ssh-keygen -l` says about them."""
import glob
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

work = tempfile.mkdtemp()
made = []
for kind, bits in (("ed25519", None), ("rsa", None), ("rsa", 4096),
                   ("ecdsa", None), ("ecdsa", 521)):
    path = os.path.join(work, f"k_{kind}_{bits}")
    argv = ["ssh-keygen", "-q", "-t", kind, "-N", "", "-C",
            f"test-{kind}@example", "-f", path]
    if bits:
        argv += ["-b", str(bits)]
    if subprocess.run(argv, capture_output=True).returncode == 0:
        made.append(path + ".pub")
# One key with no comment at all, which `ssh-keygen` prints as "no comment".
plain = os.path.join(work, "k_plain")
if subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "",
                   "-f", plain], capture_output=True).returncode == 0:
    made.append(plain + ".pub")
if not made:
    print("ssh-keygen generated nothing; skipped"); sys.exit(0)

argv = []
expected = []
labels = []


def add(args, want, label):
    argv.extend(args)
    expected.append(want)
    labels.append(label)


for path in made:
    line = open(path).read().strip()
    name = os.path.basename(path)
    listed = subprocess.run(["ssh-keygen", "-lf", path], capture_output=True,
                            text=True)
    add(["describe", line], listed.stdout.strip(), f"describe {name}")
    md5 = subprocess.run(["ssh-keygen", "-lf", path, "-E", "md5"],
                         capture_output=True, text=True)
    add(["md5", line], md5.stdout.split()[1], f"md5 {name}")
    add(["render", line], line, f"render {name}")

base = open(made[0]).read().strip()
for options, want in (
        ("no-pty,no-agent-forwarding", "no-pty\x01no-agent-forwarding"),
        ('command="echo hi,there",no-pty', 'command="echo hi,there"\x01no-pty'),
        ('from="10.0.0.1,10.0.0.2"', 'from="10.0.0.1,10.0.0.2"'),
        ('environment="A=b"', 'environment="A=b"')):
    add(["options", options + " " + base], want, f"options {options}")

for bad in ("", "#comment", "ssh-ed25519", "ssh-ed25519 !!!notbase64!!!",
            "ssh-rsa " + base.split()[1] + " type does not match blob"):
    add(["describe", bad], "error", f"refuse {bad!r}")

ran = subprocess.run([GRASM, "run", "packages/sktest/src/main.gr", "--"]
                     + argv, cwd=ROOT, capture_output=True, text=True)
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
            print(f"  {labels[i]}")
            print(f"    ssh-keygen {want}")
            print(f"    grasm      {mine}")
print(f"keys: {len(made)}   cases: {len(expected)}   agreeing: {agree}   "
      f"differing: {differ}")
sys.exit(1 if differ else 0)
