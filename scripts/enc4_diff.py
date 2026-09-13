"""base32hex, file modes and HTML entities against Python's own answers."""
import base64
import html
import os
import random
import stat
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/enctest4/src/main.gr")

random.seed(11)
args = []
expected = []
labels = []


def add(argv, want, label):
    args.extend(argv)
    expected.append(want)
    labels.append(label)


for n in list(range(0, 40)) + [100, 255]:
    raw = bytes(random.randrange(256) for _ in range(n))
    text = base64.b32hexencode(raw).decode()
    add(["b32h", ":" + raw.hex()], text, f"encode {n} bytes")
    add(["b32hn", ":" + raw.hex()], text.rstrip("="), f"unpadded {n} bytes")
    add(["b32hd", ":" + text], raw.hex(), f"decode {text[:16]}")

# Encodings that have to be refused: wrong length, padding in the middle,
# a character outside the alphabet, and bits set past the last whole byte.
for bad in ("ABC", "A=======", "0000000=", "========", "0000000V",
            "A=B=====", "01234567890123456", "0!======"):
    try:
        want = base64.b32hexdecode(bad).hex()
    except Exception:
        want = "bad"
    add(["b32hd", ":" + bad], want, f"reject {bad!r}")

for mode in (0o100644, 0o100755, 0o40755, 0o120777, 0o140755, 0o10600,
             0o20600, 0o60600, 0o4755, 0o4655, 0o2755, 0o2655, 0o1777,
             0o1666, 0o104755, 0o102755, 0o101777, 0, 0o777, 0o7777,
             0o107777):
    add(["mode", str(mode)], stat.filemode(mode), f"mode {mode:o}")
    add(["octal", str(mode)], "%04o" % (mode & 0o7777), f"octal {mode:o}")

for text in ("&amp;", "&amp", "&ampx", "&#38;", "&#x26;", "&#38", "&notit;",
             "&notin;", "&NotEqualTilde;", "a &lt; b &gt; c", "&#0;",
             "&#13;", "&#147;", "&#x9f;", "&#xD800;", "&#x110000;", "&#1;",
             "&#xfdd0;", "&#xFFFE;", "&nbsp;",
             "&CounterClockwiseContourIntegral;", "&fjlig;", "&#", "&#x",
             "&", "&;", "& amp;", "&am<p;", "&Alpha", "&alpha",
             "plain text", "&&&amp;&&",
             "&thisisaverylongentitynamethatgoespastthirtytwochars;",
             "&hellip&mdash&rsquo", "&#x41;&#x42;", "&#65&#66", "&AMP;&AMP",
             "&lang;&rang;", "&lrm;&rlm;", "&#x2A6D6;", "&#x10FFFF;",
             "&#x110001;"):
    add(["unescape", ":" + text.encode().hex()],
        html.unescape(text).encode().hex(), f"unescape {text!r}")
add(["names", "-"], "2231", "table size")

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
            print("  case:", labels[i])
            print("    python", repr(want))
            print("    grasm ", repr(mine))
print(f"cases: {len(expected)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
