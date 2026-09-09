"""git config files, judged by git itself."""
import random
import tempfile
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/gctest/src/main.gr")

random.seed(163)
SECTIONS = ["core", "user", "alias", "color", "PACK"]
SUBS = ["origin", "upstream", "My Remote", "a.b"]
KEYS = ["name", "email", "url", "fetch", "ui", "autocrlf", "CO", "lg"]
VALUES = ['plain', '"quoted value"', 'a b', '"trailing  "', 'x ; not comment',
          '"semi ; inside"', 'a\\tb', 'a\\nb', 'true', '0', '',
          '"a \\"q\\" b"', 'end\\', 'with#hash']
def document():
    lines = []
    for _ in range(random.randint(1, 5)):
        section = random.choice(SECTIONS)
        if random.random() < 0.4:
            lines.append(f'[{section} "{random.choice(SUBS)}"]')
        else:
            lines.append(f"[{section}]")
        for _ in range(random.randint(1, 4)):
            key = random.choice(KEYS)
            if random.random() < 0.15:
                lines.append(f"\t{key}")
                continue
            value = random.choice(VALUES)
            if value == 'end\\':
                lines.append(f"\t{key} = first \\")
                lines.append("\t\tsecond")
                continue
            lines.append(f"\t{key} = {value}")
        if random.random() < 0.2:
            lines.append("# a comment")
    return "\n".join(lines) + "\n"

docs = [document() for _ in range(200)]
argv = [d.encode("utf-8").hex() for d in docs]
got = subprocess.run([GRASM, "run", HARNESS, "--"] + argv,
                     capture_output=True, text=True)
if got.returncode != 0:
    print(got.stderr[-2000:]); sys.exit(1)
lines = got.stdout.split("\n")
work = tempfile.mkdtemp()
path = os.path.join(work, "config")
agree = differ = shown = skipped = 0
for i, doc in enumerate(docs):
    open(path, "w").write(doc)
    ran = subprocess.run(["git", "config", "--file", path, "--list"],
                         capture_output=True, text=True)
    if ran.returncode != 0:
        skipped += 1
        continue
    want = ran.stdout
    mine = bytes.fromhex(lines[i]).decode("utf-8") if lines[i] != "error" else "error"
    if mine == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 5:
            print("  doc:", repr(doc[:200]))
            print("    git  ", repr(want[:250]))
            print("    grasm", repr(mine[:250]))
print(f"files: {len(docs)}   agreeing: {agree}   differing: {differ}   "
      f"git refused: {skipped}")

sys.exit(1 if differ else 0)
