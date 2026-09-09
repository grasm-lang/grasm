"""Encoded words against Python's `email.header.decode_header`.

The input is an *unfolded* header, which is what a mail reader has after
joining continuation lines, so the generated words carry no newlines. An
empty piece with no charset carries nothing and is dropped on both sides:
for an empty header Python answers one empty word and this answers none.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/mwtest/src/main.gr")

import base64
import quopri
import random
from email.errors import HeaderParseError
from email.header import decode_header
random.seed(331)
TEXTS = ["hello", "échec et mat", "日本語", "a" * 90, "with  spaces",
         "under_score", "=not a word=", "café"]
CHARSETS = ["utf-8", "iso-8859-1", "us-ascii", "koi8-r"]
def word(text, charset, how):
    try:
        raw = text.encode(charset)
    except Exception:
        return None
    if how == "B":
        return f"=?{charset}?B?{base64.b64encode(raw).decode()}?="
    body = quopri.encodestring(raw, header=True).decode().replace("\n", "")
    return f"=?{charset}?Q?{body}?="
cases = ["plain header text",
         "=?utf-8?B?YQ==?= =?utf-8?B?Yg==?=",
         "=?utf-8?B?YQ==?=  text  =?utf-8?B?Yg==?=",
         "before =?utf-8?Q?caf=C3=A9?= after",
         "=?iso-8859-1?Q?caf=E9?=",
         "=?utf-8?Q?under=5Fscore?=",
         "=?utf-8?Q?a_b?=",
         "=?utf-8*en?B?aGVsbG8=?=",
         "=?utf-8?X?zzz?=",
         "=?utf-8?B?not base64!?=",
         "=? utf-8?B?YQ==?=",
         "no words at all",
         "=?utf-8?B?YQ==?=tail"]
for _ in range(400):
    parts = []
    for _ in range(random.randint(1, 4)):
        if random.random() < 0.5:
            one = word(random.choice(TEXTS), random.choice(CHARSETS),
                       random.choice("BQ"))
            if one:
                parts.append(one)
        else:
            parts.append(random.choice(["plain", "text here", "x"]))
    cases.append(" ".join(parts))
lines = []
STEP = 400
for at in range(0, len(cases), STEP):
    chunk = cases[at:at + STEP]
    got = subprocess.run([GRASM, "run", HARNESS, "--"] +
                         [c.encode().hex() for c in chunk],
                         capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-2000:]); sys.exit(1)
    lines += got.stdout.split("\n")[:len(chunk)]
# RFC 2047 says an encoded word contains no whitespace at all; Python
# accepts one inside the charset. This reads the word as the plain text
# it is, and the case is named rather than quietly passed.
LOOSER_THAN_THE_RFC = {"=? utf-8?B?YQ==?="}
agree = differ = shown = refused = lenient = 0
for i, text in enumerate(cases):
    pieces = []
    try:
        decoded = decode_header(text)
    except HeaderParseError:
        # Python refuses a word whose base64 is malformed; this reads it
        # as the plain text it looks like, which is what a mail reader
        # has to do with a header it did not write.
        refused += 1
        continue
    for raw, charset in decoded:
        if isinstance(raw, str):
            raw = raw.encode()
        if not raw and not charset:
            continue
        pieces.append(raw.hex() + ":" + (charset or ""))
    want = ",".join(pieces)
    if lines[i] == want:
        agree += 1
    elif text in LOOSER_THAN_THE_RFC:
        lenient += 1
        print(f"  python accepts whitespace inside an encoded word; "
              f"RFC 2047 does not -- {text!r}")
    else:
        differ += 1; shown += 1
        if shown <= 8:
            print(f"  {text!r}\n    python {want}\n    grasm  {lines[i]}")
print(f"headers: {len(cases)}   agreeing: {agree}   differing: {differ}   "
      f"python refused: {refused}   python more lenient: {lenient}")
sys.exit(1 if differ else 0)
