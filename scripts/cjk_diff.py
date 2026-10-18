"""Johab, CP950, JIS X 0213 and JIS X 0212 against Python's codecs.

For each codec every byte, every pair a lead byte can start (refusals
included, so a decoder that accepts too much is caught), every three-byte
EUC sequence, and the characters they decode to encoded back. Then every
code point Python's encoder writes that no decode produced (the
encode-only characters a reverse table built from the forward one misses),
text with a letter and a combining mark that has a cell of its own, and a
sample of code points Python refuses.

- `johab`, `cp950`: the packages of the same name.
- `shift_jis_2004`, `euc_jis_2004`, `shift_jisx0213`, `euc_jisx0213`:
  `jisx0213` in its four forms.
- `euc_jp`: `jisx0212`'s EUC-JP, which carries JIS X 0212 beside
  `packages/shiftjis`'s JIS X 0208.
"""
import os
import random
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
REFUSED = "?refused?".encode().hex()
rng = random.Random(20261018)

stale = subprocess.run([sys.executable, "scripts/gen_jis.py", "--check"], cwd=ROOT, capture_output=True, text=True)
if stale.returncode != 0:
    print(stale.stdout + stale.stderr); sys.exit(1)

work = tempfile.mkdtemp(prefix="cjkdiff")
harness = os.path.join(work, "cjktest")
built = subprocess.run([GRASM, "build", "packages/cjktest/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

CODE_POINTS = list(range(0x80, 0xD800)) + list(range(0xE000, 0x10000)) + list(range(0x20000, 0x2B820))


def decoded(raw, codec):
    try:
        return raw.decode(codec)
    except Exception:
        return None


def encoded(text, codec):
    try:
        return text.encode(codec).hex()
    except Exception:
        return REFUSED


def corpus(codec):
    cases = []
    seen = set()

    def dec(raw, label):
        text = decoded(raw, codec)
        cases.append(("dec", raw.hex(), REFUSED if text is None else text.encode().hex(), label))
        if text is not None and text not in seen and any(ord(c) >= 0x80 for c in text):
            seen.add(text)
            cases.append(("enc", text.encode().hex(), encoded(text, codec), f"encode {text!r}"))

    for b in range(256):
        dec(bytes([b]), f"byte {b:02x}")
    if codec.startswith("euc"):
        for lead in range(0xA1, 0xFF):
            for trail in range(0x30, 0x100):
                dec(bytes([lead, trail]), f"pair {lead:02x}{trail:02x}")
        for b in range(0x80, 0x100):
            dec(bytes([0x8E, b]), f"kana 8e{b:02x}")
        for row in range(0xA0, 0x100):
            for cell in range(0xA0, 0x100):
                dec(bytes([0x8F, row, cell]), f"triple 8f{row:02x}{cell:02x}")
    else:
        for lead in range(0x80, 0x100):
            for trail in range(0x30, 0x100):
                dec(bytes([lead, trail]), f"pair {lead:02x}{trail:02x}")
    refused = []
    for cp in CODE_POINTS:
        ch = chr(cp)
        if ch in seen:
            continue
        want = encoded(ch, codec)
        if want != REFUSED:
            cases.append(("enc", ch.encode().hex(), want, f"encode-only U+{cp:04X}"))
        else:
            refused.append(ch)
    for ch in rng.sample(refused, min(300, len(refused))):
        cases.append(("enc", ch.encode().hex(), REFUSED, f"refused U+{ord(ch):04X}"))
    for ch in map(chr, range(0x80)):
        cases.append(("enc", ch.encode().hex(), encoded(ch, codec), f"ascii {ch!r}"))
    texts = ["", "hello, world", "abc\\~def", "日本語のテキスト", "한국어 텍스트", "繁體中文", "か゚き゚",
             "か゚x", "˩˥˥˩", "æ̀", "゚", "ｱｲｳ", "€ 100", "¥‾"]
    for t in texts + ["".join(rng.choice(sorted(seen)) for _ in range(12)) for _ in range(40)]:
        want = encoded(t, codec)
        cases.append(("enc", t.encode().hex(), want, f"text {t!r}"))
        if want != REFUSED:
            back = decoded(bytes.fromhex(want), codec)
            cases.append(("dec", want, back.encode().hex(), f"text back {t!r}"))
    if codec.startswith("euc"):
        for raw in (b"\x8f", b"\x8f\xb0", b"A\xa4", b"\x8e"):
            dec(raw, f"truncated {raw.hex()}")
    else:
        dec(b"A\x81", "truncated lead")
    return cases


CODECS = ["johab", "cp950", "shift_jis_2004", "euc_jis_2004", "shift_jisx0213", "euc_jisx0213", "euc_jp"]
CHUNK = 2500
families = {}
shown = 0
total = 0
for codec in CODECS:
    cases = corpus(codec)
    lines = []
    for start in range(0, len(cases), CHUNK):
        argv = [codec]
        for op, data, _, _ in cases[start:start + CHUNK]:
            argv += [op, ":" + data]
        ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=300)
        if ran.returncode != 0:
            print(f"{codec}: harness failed: {ran.stderr[-1500:]}"); sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
    fam = families.setdefault(codec, [0, 0])
    for i, (op, data, want, label) in enumerate(cases):
        mine = lines[i] if i < len(lines) else "<missing>"
        if mine == want:
            fam[0] += 1
        else:
            fam[1] += 1
            shown += 1
            if shown <= 12:
                show = (lambda h: bytes.fromhex(h).decode("utf-8", "replace") if op == "dec" and h != REFUSED else h)
                try:
                    print(f"  {codec} {label}: python {show(want)!r}  grasm {show(mine)!r}")
                except Exception:
                    print(f"  {codec} {label}: python {want}  grasm {mine}")
    total += len(cases)
agree = sum(a for a, _ in families.values())
differ = sum(d for _, d in families.values())
bad = [f"{k}:{d}" for k, (a, d) in families.items() if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 10000]
if thin or len(families) != len(CODECS):
    print(f"too few cases to mean anything: {thin}"); sys.exit(1)
print(f"cases: {total}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
