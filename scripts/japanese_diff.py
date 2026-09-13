"""EUC-JP and ISO-2022-JP against Python's codecs, both ways, every character.

The corpus is every two-byte character all three Japanese codecs in this tree
agree exists — found by round-tripping each candidate byte pair rather than
by listing them here — so the table is checked entry by entry rather than at
a handful of sample points.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))

characters = []
for lead in list(range(0x81, 0xA0)) + list(range(0xE0, 0xF0)):
    for trail in list(range(0x40, 0x7F)) + list(range(0x80, 0xFD)):
        try:
            character = bytes([lead, trail]).decode("shift_jis")
        except Exception:
            continue
        try:
            character.encode("euc_jp")
            character.encode("iso2022_jp")
        except Exception:
            continue
        characters.append(character)

cases = []
for character in characters:
    cases.append(("eucdec", character.encode("euc_jp"), character.encode()))
    cases.append(("eucenc", character.encode(), character.encode("euc_jp")))
    cases.append(("isodec", character.encode("iso2022_jp"),
                  character.encode()))
    cases.append(("isoenc", character.encode(),
                  character.encode("iso2022_jp")))

for text in ("", "hello", "日本語", "abc日本def", "ｱｲｳｴｵ", "半角ｶﾅ混じり",
             "改行\nあり\n", "①②③"):
    for encoding, read, write in (("euc_jp", "eucdec", "eucenc"),
                                  ("iso2022_jp", "isodec", "isoenc")):
        try:
            raw = text.encode(encoding)
        except Exception:
            continue
        cases.append((read, raw, text.encode()))
        cases.append((write, text.encode(), raw))

argv = []
for command, source, _ in cases:
    argv += [command, ":" + source.hex()]
ran = subprocess.run([GRASM, "run", "packages/jptest/src/main.gr", "--"]
                     + argv, cwd=ROOT, capture_output=True, text=True)
if ran.returncode != 0:
    print(ran.stderr[-2000:]); sys.exit(1)
lines = ran.stdout.split("\n")
agree = differ = shown = 0
for i, (command, source, want) in enumerate(cases):
    mine = lines[i] if i < len(lines) else "<missing>"
    if mine == want.hex():
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 4:
            print(f"  {command} {source.hex()[:30]}")
            print(f"    python {want.hex()[:40]}")
            print(f"    grasm  {mine[:40]}")
print(f"characters: {len(characters)}   cases: {len(cases)}   "
      f"agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
