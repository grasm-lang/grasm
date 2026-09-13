"""Text for the UTF-7, UTF-32, ISO-2022-KR and HZ gate."""
import random

random.seed(67)

TEXTS = [
    "", "hello", "a+b", "~tilde~", "a~~b",
    "héllo wörld", "日本語", "한국어 텍스트", "中文文本",
    "안녕하세요", "你好世界", "Ελληνικά", "Привет",
    "mixed ASCII and 한글 and back",
    "line one\nline two\n", "tab\there", "quote \" and ' and \\",
    "\U0001f600", "a\U0001f600b", "emoji 😀 in text",
    "+-+-+", "~{not a run~}", "=?=",
]
# Some Korean and Chinese drawn from the sets each encoding can carry.
KOREAN = "가나다라마바사아자차카타파하각간갇갈감갑값갓강"
CHINESE = "中国人民共和国北京上海广州深圳天津重庆香港澳门"
for n in (1, 3, 8, 20):
    TEXTS.append(KOREAN[:n])
    TEXTS.append(CHINESE[:n])
    TEXTS.append("x" + KOREAN[:n] + "y")
    TEXTS.append("x" + CHINESE[:n] + "y")
for _ in range(30):
    TEXTS.append("".join(random.choice("abc 가나다中国~+\n")
                         for _ in range(random.randrange(0, 20))))


def build():
    argv, expected, labels = [], [], []

    def add(command, source, want, label):
        argv.extend([command, ":" + source.hex()])
        expected.append(want)
        labels.append(label)

    for text in TEXTS:
        raw = text.encode()
        add("u7enc", raw, text.encode("utf-7").hex(), f"utf7 encode {text!r}")
        add("u7dec", text.encode("utf-7"), raw.hex(),
            f"utf7 decode {text!r}")
        add("u32le", raw, text.encode("utf-32-le").hex(),
            f"utf32-le {text!r}")
        add("u32be", raw, text.encode("utf-32-be").hex(),
            f"utf32-be {text!r}")
        add("u32bom", raw, text.encode("utf-32").hex(), f"utf32 {text!r}")
        add("u32dec", text.encode("utf-32"), raw.hex(),
            f"utf32 decode {text!r}")
        add("u32decle", text.encode("utf-32-le"), raw.hex(),
            f"utf32-le decode {text!r}")
        for command, codec in (("krenc", "iso2022_kr"), ("hzenc", "hz")):
            try:
                want = text.encode(codec).hex()
            except Exception:
                want = "?refused?".encode().hex()
            add(command, raw, want, f"{codec} encode {text!r}")
        for command, codec in (("krdec", "iso2022_kr"), ("hzdec", "hz")):
            try:
                encoded = text.encode(codec)
            except Exception:
                continue
            add(command, encoded, raw.hex(), f"{codec} decode {text!r}")

    # Documents at the edge: some are refused and some are not, and which
    # is which comes from the codec rather than from an opinion here.
    # `+AOk` with no terminator is accepted; `+!!!-` is not.
    for command, codec, raw in (("u7dec", "utf-7", b"+AOk"),
                                ("u7dec", "utf-7", b"+!!!-"),
                                ("u7dec", "utf-7", b"+AOk!"),
                                ("u7dec", "utf-7", b"+"),
                                ("u32decle", "utf-32-le", b"abc"),
                                ("u32decle", "utf-32-le",
                                 b"\x00\xd8\x00\x00"),
                                ("u32decle", "utf-32-le",
                                 b"\x00\x00\x11\x00"),
                                ("krdec", "iso2022_kr",
                                 b"\x1b$)Chello\x80"),
                                ("hzdec", "hz", b"~{\x80\x80~}"),
                                ("hzdec", "hz", b"a~~b"),
                                ("hzdec", "hz", b"a~\nb")):
        try:
            want = raw.decode(codec).encode().hex()
        except Exception:
            want = "?refused?".encode().hex()
        add(command, raw, want, f"edge {command} {raw!r}")
    return argv, expected, labels
