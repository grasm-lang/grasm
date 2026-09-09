import mailbox
import random
import tempfile
"""mbox files against Python's own `mailbox.mbox`.

Python hands back a body with the `>From ` escaping still in it and
leaves undoing it to the caller, so the comparison is made in the `raw`
dialect, which does the same. The escaping itself is checked by the round
trip below: written back out in `mboxrd` and read again, every message
must come back identical.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/mboxtest/src/main.gr")

random.seed(347)
BODIES = [
    "a plain body\n",
    "From the start of a line\nand more\n",
    ">From an escaped line\n",
    ">>From a doubly escaped line\n",
    "line one\n\nline three after a blank\n",
    "",
    "trailing spaces   \nand a tab\there\n",
]
HEADERS = [
    ("Subject", "hello"),
    ("Subject", "a very long subject that will be folded by the writer"),
    ("From", "Ada <ada@example.test>"),
    ("To", "b@x.test, c@x.test"),
    ("Date", "Mon, 1 Jan 2024 00:00:00 +0000"),
    ("X-Custom", "value"),
]
def message(i):
    lines = [f"From sender{i}@example.test Mon Jan  1 00:00:0{i % 10} 2024"]
    for name, value in random.sample(HEADERS, random.randint(2, 5)):
        lines.append(f"{name}: {value}")
    lines.append("")
    body = random.choice(BODIES)
    if body:
        # Escape the way a writer must, so the file is well formed.
        for line in body.rstrip("\n").split("\n"):
            stripped = line.lstrip(">")
            lines.append(">" + line if stripped.startswith("From ") else line)
    lines.append("")
    return "\n".join(lines) + "\n", body
work = tempfile.mkdtemp()
agree = differ = 0
for round_number in range(60):
    parts = [message(i) for i in range(random.randint(1, 4))]
    text = "".join(part for part, _ in parts)
    path = os.path.join(work, f"box{round_number}.mbox")
    open(path, "w").write(text)
    got = subprocess.run([GRASM, "run", HARNESS, "--", path, "raw"],
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-1500:]); sys.exit(1)
    raw = got.stdout.strip()
    mine = bytes.fromhex(raw).decode() if raw else ""
    trip = subprocess.run([GRASM, "run", HARNESS, "--", path, "mboxrd",
                           "again"], capture_output=True, text=True)
    if trip.stdout.strip() != "round trip holds":
        print(f"  round trip: {trip.stdout.strip()!r}")
        differ += 1
        continue
    box = mailbox.mbox(path)
    wanted = []
    for key in box.keys():
        item = box[key]
        sender, _, date = item.get_from().partition(" ")
        headers = "\x04".join(f"{name}={value}" for name, value in
                              item.items())
        body = item.get_payload()
        wanted.append(f"{sender}\x03{date.strip()}\x03{headers}\x03{body}")
    box.close()
    want = "\x01".join(wanted)
    if mine == want:
        agree += 1
    else:
        differ += 1
        if differ <= 3:
            at = next((k for k in range(min(len(want), len(mine)))
                       if want[k] != mine[k]), min(len(want), len(mine)))
            print(f"  first difference at {at} of {len(want)}/{len(mine)}")
            print("    python", repr(want[max(0, at - 60):at + 60]))
            print("    grasm ", repr(mine[max(0, at - 60):at + 60]))
print(f"mailboxes: {agree + differ}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
