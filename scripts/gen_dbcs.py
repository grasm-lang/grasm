"""Generate a double-byte codec package from Python's own codec.

Every table in the packages this writes comes from decoding and encoding
with Python's `codecs`, not from a published mapping file typed out by hand:
the decode rows are what Python decodes each byte pair to, the single bytes
are what it decodes each lone byte to, and the encoder preferences are the
cases where Python's encoder picks a pair other than the first one that
decodes to the character. That last list is the one a hand-built table
always gets wrong.

    python3 scripts/gen_dbcs.py cp932 Cp932 packages/cp932/src/main.gr header.txt
"""
import sys


def build(codec, prefix, header):
    singles = {}
    for b in range(0x80, 0x100):
        try:
            text = bytes([b]).decode(codec)
        except Exception:
            continue
        if len(text) == 1:
            singles[b] = ord(text)

    rows = {}
    multi = {}
    first = {}
    wide = False
    for lead in range(0x80, 0x100):
        if lead in singles:
            continue
        row = []
        any_valid = False
        for trail in range(0x40, 0x100):
            try:
                text = bytes([lead, trail]).decode(codec)
            except Exception:
                row.append(0)
                continue
            if len(text) == 1:
                row.append(ord(text))
                any_valid = True
                if ord(text) > 0xFFFF:
                    wide = True
                if text not in first:
                    first[text] = (lead, trail)
            else:
                row.append(0)
                multi[(lead, trail)] = [ord(c) for c in text]
                any_valid = True
        if any_valid:
            rows[lead] = row

    overrides = {}
    for cp in list(range(0x80, 0x10000)) + list(range(0x20000, 0x32000)):
        if 0xD800 <= cp <= 0xDFFF:
            continue
        ch = chr(cp)
        try:
            encoded = ch.encode(codec)
        except Exception:
            continue
        if len(encoded) != 2:
            continue
        pair = (encoded[0], encoded[1])
        if first.get(ch) != pair:
            overrides[cp] = pair

    width = 6 if wide else 4
    leads = sorted(rows)
    out = [header.rstrip("\n"), ""]
    out.append("type %sError =" % prefix)
    out.append("    | %sInvalid(at: i64, byte: i64)" % prefix)
    out.append("    | %sTruncated(at: i64)" % prefix)
    out.append("    | %sUnencodable(at: i64, codepoint: i64)" % prefix)
    out.append("")
    out.append("/// A prepared reverse table, built once for a document rather than")
    out.append("/// searched for every character.")
    out.append("type %sEncoder = { map: Map[i64, i64] }" % prefix)
    out.append("")
    out.append("let WIDTH = %d" % width)
    out.append("")
    out.append("fn leads() -> [i64] {")
    out.append("    return [%s]" % ", ".join(str(l) for l in leads))
    out.append("}")
    out.append("")
    out.append("/// One row per lead byte, one entry per trail byte from 0x40 to 0xFF,")
    out.append("/// %d hexadecimal digits each, 0 where the pair is not a character." % width)
    out.append("fn rows() -> [str] {")
    out.append("    return [")
    for lead in leads:
        text = "".join(("%0" + str(width) + "X") % v for v in rows[lead])
        out.append('        "%s",' % text)
    out.append("    ]")
    out.append("}")
    out.append("")
    def packed(name, doc, entries):
        body = " ".join(entries)
        out.append(doc)
        out.append("fn %s() -> str {" % name)
        if not body:
            out.append('    return ""')
        else:
            pieces = []
            line = ""
            for word in body.split(" "):
                if line and len(line) + 1 + len(word) > 60:
                    pieces.append(line)
                    line = word
                else:
                    line = (line + " " + word) if line else word
            if line:
                pieces.append(line)
            out.append('    return "%s "' % pieces[0])
            for piece in pieces[1:]:
                out[-1] = out[-1] + " +"
                out.append('        "%s "' % piece)
        out.append("}")
        out.append("")

    packed("packed_singles",
           "/// The bytes that are a character on their own above 0x7F, as\n"
           "/// `byte:codepoint` in hexadecimal.",
           ["%x:%x" % kv for kv in sorted(singles.items())])
    packed("packed_sequences",
           "/// Pairs that decode to more than one code point, as\n"
           "/// `pair:codepoint,codepoint`.",
           ["%x:%s" % (l * 256 + t, ",".join("%x" % c for c in cps))
            for (l, t), cps in sorted(multi.items())])
    packed("packed_preferred",
           "/// Code points whose encoding is not the first pair that decodes to\n"
           "/// them, taken from what the reference encoder actually writes, as\n"
           "/// `codepoint:pair`.",
           ["%x:%x" % (cp, l * 256 + t) for cp, (l, t) in sorted(overrides.items())])
    return "\n".join(out) + "\n", len(leads), sum(
        1 for l in rows for v in rows[l] if v), len(singles), len(multi), \
        len(overrides)


if __name__ == "__main__":
    codec, prefix, target, header_path = sys.argv[1:5]
    header = open(header_path).read()
    text, leads, pairs, singles, multi, overrides = build(codec, prefix,
                                                          header)
    open(target, "w").write(text)
    print(f"{codec}: {leads} leads, {pairs} pairs, {singles} single bytes, "
          f"{multi} sequences, {overrides} encoder preferences")
