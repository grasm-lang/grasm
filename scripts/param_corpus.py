"""Header parameters, dumb databases and MIME base64, for one gate."""
import base64
import dbm.dumb
import os
import random
import shutil
import tempfile
from email.message import Message
from email.utils import collapse_rfc2231_value

HEADERS = [
    ('attachment; filename="report.pdf"', "filename"),
    ("attachment; filename=report.pdf", "filename"),
    ("attachment; filename*=UTF-8''na%C3%AFve.txt", "filename"),
    ("attachment; filename*=utf-8''%D0%BE%D1%82%D1%87%D1%91%D1%82.pdf",
     "filename"),
    ("attachment; filename*=iso-8859-1''na%EFve.txt", "filename"),
    ("attachment; filename*0*=UTF-8''%D0%BE; filename*1*=%D1%82",
     "filename"),
    ('attachment; filename*0="first "; filename*1="second"', "filename"),
    ("attachment; filename*0*=UTF-8''caf; filename*1=e; filename*2*=%CC%81",
     "filename"),
    ("attachment; filename*=UTF-8'en'hello.txt", "filename"),
    ('inline; filename=a.txt; creation-date="Wed, 1 Jan 2020"',
     "creation-date"),
    ("text/plain; charset=utf-8", "charset"),
    ("text/plain; charset*=UTF-8''utf-8", "charset"),
    ('attachment; filename="quoted \\"name\\".txt"', "filename"),
    ("attachment", "filename"),
    ("attachment; filename*=unknown-8bit''%41%42", "filename"),
    ('form-data; name="file"; '
     "filename*=UTF-8''%E6%97%A5%E6%9C%AC.txt", "filename"),
    # An extended value with no charset at all, which is malformed and
    # which the receiving end reads as ISO-8859-1.
    ("attachment; filename*=%E6%97%A5.txt", "filename"),
]


def build():
    random.seed(83)
    argv, expected, labels = [], [], []

    def add(args, want, label):
        argv.extend(args)
        expected.append(want)
        labels.append(label)

    for header, name in HEADERS:
        message = Message()
        message["Content-Disposition"] = header
        value = message.get_param(name, header="content-disposition")
        if value is None:
            want = "?missing?"
        elif isinstance(value, tuple):
            try:
                want = collapse_rfc2231_value(value)
            except Exception:
                want = "?missing?"
        else:
            want = value
        add(["get", ":" + header.encode().hex(), ":" + name.encode().hex()],
            want.encode().hex(), f"parameter {name} of {header[:40]!r}")
        add(["head", ":" + header.encode().hex(), ":" + b"-".hex()],
            header.split(";")[0].strip().encode().hex(),
            f"head of {header[:40]!r}")

    # MIME base64, against the functions that wrap at 76 columns.
    payloads = [b"", b"a", b"ab", b"abc", b"hello world", bytes(range(256)),
                b"x" * 100, b"y" * 1000,
                bytes(random.randrange(256) for _ in range(57)),
                bytes(random.randrange(256) for _ in range(58)),
                bytes(random.randrange(256) for _ in range(4096))]
    for raw in payloads:
        add(["b64enc", ":" + raw.hex()],
            base64.encodebytes(raw).decode().encode().hex(),
            f"encode {len(raw)} bytes")
        add(["b64dec", ":" + base64.encodebytes(raw).decode().encode().hex()],
            raw.hex(), f"decode {len(raw)} bytes")
    # Wrapped by somebody else, or not wrapped at all, or missing padding.
    for text, raw in (("aGVsbG8=", b"hello"), ("aGVsbG8", b"hello"),
                      ("aGVs\r\nbG8=", b"hello"), ("  aGVs bG8=  ", b"hello"),
                      ("aGVsbG8=\n\n\n", b"hello"), ("", b"")):
        add(["b64dec", ":" + text.encode().hex()], raw.hex(),
            f"decode {text!r}")
    for text in ("aGV!bG8=", "a"):
        add(["b64dec", ":" + text.encode().hex()],
            "?refused?".encode().hex(), f"refuse {text!r}")

    # A dumb database written by `dbm.dumb` itself.
    work = tempfile.mkdtemp()
    try:
        path = os.path.join(work, "store")
        database = dbm.dumb.open(path, "n")
        pairs = [(b"alpha", b"one"), (b"beta", b"two and more"),
                 (b"gam ma", b"x" * 300), (b"quote'key", b"v"),
                 (b"\xff\xfe", b"high bytes"), (b"comma,key", b"w"),
                 (b"", b"empty key"), (b"big", bytes(range(256)) * 4)]
        for key, value in pairs:
            database[key] = value
        database.close()
        # The index is read as bytes: `repr` escapes everything outside
        # printable ASCII, so it is ASCII in practice, and reading it as
        # text would hide the case where it is not.
        index = open(path + ".dir", "rb").read()
        data = open(path + ".dat", "rb").read()
    finally:
        shutil.rmtree(work, ignore_errors=True)
    add(["dbmkeys", ":" + index.hex()],
        ",".join(key.hex() for key, _ in pairs), "every key")
    # Written back byte for byte only where the keys are ASCII: `repr`
    # leaves a printable Latin-1 character as itself and `render_key`
    # escapes it, which is a different valid repr of the same key.
    ascii_only = b"\n".join(
        line for line in index.split(b"\n")
        if line.strip() and all(b < 0x80 for b in line))
    add(["dbmline", ":" + ascii_only.hex()], ascii_only.hex(),
        "an ASCII index written back unchanged")
    for key, value in pairs:
        add(["dbmget", ":" + index.hex(), ":" + key.hex(),
             ":" + data.hex()], value.hex(), f"value for {key!r}")
    return argv, expected, labels
