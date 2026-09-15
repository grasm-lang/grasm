"""Every character CP932, CP949 and Big5-HKSCS know, plus every EBCDIC byte.

Characters are found by decoding each candidate byte pair with Python's own
codec, so the corpus covers the whole table rather than sample points, and
each one is checked in both directions against that codec.
"""

CODECS = ["cp932", "cp949", "big5hkscs"]
EBCDIC_PAGES = ["cp037", "cp273", "cp500", "cp1026", "cp1140"]


def build():
    argv, expected, labels = [], [], []

    def add(command, source, want, label):
        argv.extend([command, ":" + source.hex()])
        expected.append(want)
        labels.append(label)

    for codec in CODECS:
        seen = set()
        for lead in range(0x80, 0x100):
            for trail in range(0x40, 0x100):
                raw = bytes([lead, trail])
                try:
                    text = raw.decode(codec)
                except Exception:
                    continue
                add(codec + ".dec", raw, text.encode().hex(),
                    f"{codec} decode {raw.hex()}")
                if text in seen:
                    continue
                seen.add(text)
                add(codec + ".enc", text.encode(),
                    text.encode(codec).hex(), f"{codec} encode {text!r}")
        # Characters that encode without ever being decoded to — CP932's
        # six Microsoft spellings of JIS characters are the famous ones.
        for cp in list(range(0x80, 0x3100)) + list(range(0xFF00, 0x10000)):
            ch = chr(cp)
            if ch in seen or 0xD800 <= cp <= 0xDFFF:
                continue
            try:
                encoded = ch.encode(codec)
            except Exception:
                continue
            add(codec + ".enc", ch.encode(), encoded.hex(),
                f"{codec} encode-only {ch!r}")
        # Text, including runs of ASCII and a truncated final lead byte.
        for sample in ("", "hello", "abcあdef", "日本"):
            try:
                encoded = sample.encode(codec)
            except Exception:
                continue
            add(codec + ".enc", sample.encode(), encoded.hex(),
                f"{codec} text {sample!r}")
            add(codec + ".dec", encoded, sample.encode().hex(),
                f"{codec} text back {sample!r}")
        first_lead = None
        for lead in range(0x81, 0x100):
            try:
                bytes([lead]).decode(codec)
            except Exception:
                first_lead = lead
                break
        if first_lead is not None:
            add(codec + ".dec", bytes([0x41, first_lead]),
                "?refused?".encode().hex(), f"{codec} truncated pair")
        add(codec + ".enc", "\U0001f600".encode(),
            "?refused?".encode().hex(), f"{codec} emoji is unencodable")

    for page in EBCDIC_PAGES:
        raw = bytes(range(256))
        add("ebcdic.%s.dec" % page, raw, raw.decode(page).encode().hex(),
            f"{page} all 256 bytes")
        text = raw.decode(page)
        add("ebcdic.%s.enc" % page, text.encode(), text.encode(page).hex(),
            f"{page} all 256 characters back")
        add("ebcdic.%s.enc" % page, "HELLO, World 123".encode(),
            "HELLO, World 123".encode(page).hex(), f"{page} a sentence")
    return argv, expected, labels
