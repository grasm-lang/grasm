"""The names the IDNA gate checks against Python's `encodings.idna`.

RFC 3491 names Unicode 3.2 for the normalisation step and Python keeps a
copy of those tables beside the current ones; `unicodenorm` is current. A
name the two versions normalise differently is counted and left out rather
than reported as a failure of either — the count is printed, so the
divergence stays visible.
"""
import unicodedata

NAMES = [
    "münchen.de", "bücher.example", "日本語.jp",
    "example.com", "räksmörgås.se",
    "例え.テスト",
    "почта.рф",
    "مثال.إختبار",
    "उदाहरण.परीक्"
    "षा",
    "ελληνικά.gr",
    "한국.kr", "MÜNCHEN.DE", "Example.COM", "a.b.c",
    "xn--mnchen-3ya.de", "www.münchen.de", "münchen.de.",
    "3年B組金八先生.jp",
    "ünïcödé.example", "ÅÄÖ.se",
    "sub.domain.example.org",
    # Three of the four separators a domain may use are not a full stop.
    "münchen。de", "test.ＣＯＭ",
]
REFUSED = ["", ".", "a b.com", "münchen .de", "xn--.com",
           "a" * 70 + ".com", " .com", "xn--a-ecp.ru",
           # Characters that change how the rest of the label renders, and
           # the ones that merely look like spaces and do not.
           "a\u200eb.com", "a\u202ab.com", "a\u2028b.com",
           "a\ue000b.com", "a\ufffeb.com", "a\u3000b.com",
           "a\u2000b.com", "a\u0007b.com"]
LABELS = ["MÜNCHEN", "Bücher", "ẞ", "ﬁ", "Ⅻ",
          "①", "ｆｕｌｌ", "ς", "ß", "ABC",
          # A soft hyphen and a zero-width space: invisible, and they must
          # not make two names different.
          "a­b", "​x"]


def comparable(name):
    return (unicodedata.normalize("NFKC", name)
            == unicodedata.ucd_3_2_0.normalize("NFKC", name))


PUNYCODE_LABELS = [
    "münchen", "bücher", "日本語",
    "räksmörgås", "例え",
    "почта", "مثال",
    "ελληνικά",
    "한국", "xn", "abc", "", "a",
    "3年B組金八先生",
    "Mājas", "☃", "\U0001f600", "a-b", "-", "--", "ab-cd",
    "ünïcödé",
]
PUNYCODE_REFUSED = ["!!!", "999999999999", "aaaaaaaaaaaaaaaaaaaaaa"]


def build_punycode():
    """Labels for the punycode harness, which the same gate also runs."""
    argv, expected, labels = [], [], []
    for label in PUNYCODE_LABELS:
        try:
            encoded = label.encode("punycode").decode("ascii")
        except Exception:
            continue
        argv += ["encode", ":" + label.encode().hex()]
        expected.append(encoded.encode().hex())
        labels.append("punycode encode %r" % label)
        argv += ["decode", ":" + encoded.encode().hex()]
        expected.append(label.encode().hex())
        labels.append("punycode decode %r" % encoded)
    for bad in PUNYCODE_REFUSED:
        argv += ["decode", ":" + bad.encode().hex()]
        try:
            want = bad.encode("ascii").decode("punycode")
        except Exception:
            want = "?refused?"
        expected.append(want.encode().hex())
        labels.append("punycode decode %r" % bad)
    return argv, expected, labels


def build():
    import encodings.idna as pyidna
    argv, expected, labels, skipped = [], [], [], 0
    for name in NAMES + REFUSED:
        if not comparable(name):
            skipped += 1
            continue
        try:
            want = name.encode("idna").decode("ascii")
        except Exception:
            want = "?refused?"
        argv += ["encode", ":" + name.encode().hex()]
        expected.append(want.encode().hex())
        labels.append("encode %r" % name)
    for name in NAMES:
        if not comparable(name):
            continue
        try:
            ascii_form = name.encode("idna").decode("ascii")
        except Exception:
            continue
        try:
            want = ascii_form.encode("ascii").decode("idna")
        except Exception:
            want = "?refused?"
        argv += ["decode", ":" + ascii_form.encode().hex()]
        expected.append(want.encode().hex())
        labels.append("decode %r" % ascii_form)
    for label in LABELS:
        if not comparable(label):
            skipped += 1
            continue
        try:
            want = pyidna.nameprep(label)
        except Exception:
            want = "?refused?"
        argv += ["prep", ":" + label.encode().hex()]
        expected.append(want.encode().hex())
        labels.append("nameprep %r" % label)
    return argv, expected, labels, skipped
