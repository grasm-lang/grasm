"""The corpus the XML-RPC gate checks against Python's `xmlrpc.client`."""
import xmlrpc.client as rpc


def describe(value):
    if value is None:
        return "nil"
    if isinstance(value, bool):
        return "boolean:" + ("1" if value else "0")
    if isinstance(value, int):
        return "int:%d" % value
    if isinstance(value, float):
        return "double:" + repr(value)
    if isinstance(value, rpc.Binary):
        return "base64:" + value.data.hex()
    if isinstance(value, rpc.DateTime):
        return "date:" + value.value
    if isinstance(value, str):
        return "string:" + value
    if isinstance(value, (list, tuple)):
        return "array(" + ", ".join(describe(v) for v in value) + ")"
    if isinstance(value, dict):
        return "struct(" + ", ".join("%s=%s" % (k, describe(v))
                                     for k, v in value.items()) + ")"
    raise TypeError(value)


VALUES = [
    1, 0, -1, 2 ** 31 - 1, -(2 ** 31),
    True, False,
    "", "hello", "a < b & c > d", "café", "日本語",
    1.5, -0.25, 0.0, 100.0,
    rpc.Binary(b""), rpc.Binary(b"hello"), rpc.Binary(bytes(range(64))),
    rpc.DateTime("20200101T12:30:45"),
    [], [1, 2, 3], ["a", True, 1.5],
    {}, {"k": 1}, {"a": 1, "b": [1, 2], "c": {"d": "e"}},
    [[1, [2, [3]]]],
    None,
]

CALLS = [
    ("ping", ()),
    ("system.listMethods", ()),
    ("add", (1, 2)),
    ("mixed", (1, "two", [3], {"four": 5})),
    ("unicode", ("日本語",)),
    ("nothing", (None,)),
]


def build():
    argv, expected, labels = [], [], []

    def add(args, want, label):
        argv.extend(args)
        expected.append(want)
        labels.append(label)

    # Reading: a document Python wrote, described.
    for value in VALUES:
        raw = rpc.dumps((value,), methodresponse=True, allow_none=True)
        add(["read", ":" + raw.encode().hex()],
            ("|" + describe(value)).encode().hex(), f"read {value!r}"[:50])
    for method, params in CALLS:
        raw = rpc.dumps(params, methodname=method, allow_none=True)
        want = method + "|" + " ".join(describe(p) for p in params)
        add(["read", ":" + raw.encode().hex()], want.encode().hex(),
            f"read call {method}")
    for code, why in ((1, "bad"), (-32601, "method not found"), (0, "")):
        raw = rpc.dumps(rpc.Fault(code, why))
        want = "fault|" + describe({"faultCode": code, "faultString": why})
        add(["read", ":" + raw.encode().hex()], want.encode().hex(),
            f"read fault {code}")
    # A bare `<value>` is a string, which is the rule most often got wrong.
    bare = ("<?xml version='1.0'?><methodResponse><params><param>"
            "<value>7</value></param></params></methodResponse>")
    add(["read", ":" + bare.encode().hex()],
        "|string:7".encode().hex(), "bare value is a string")
    # `i4` is `int` under another name.
    i4 = ("<?xml version='1.0'?><methodResponse><params><param>"
          "<value><i4>7</i4></value></param></params></methodResponse>")
    add(["read", ":" + i4.encode().hex()], "|int:7".encode().hex(),
        "i4 is int")
    for broken in ("", "<notrpc/>", "<methodCall>", "not xml at all"):
        add(["read", ":" + broken.encode().hex()], "error",
            f"refuse {broken!r}")

    # Writing: the bytes Python would have written.
    for method, params in CALLS:
        payload = method + "|" + "|".join(describe(p) for p in params)
        add(["call", ":" + payload.encode().hex()],
            rpc.dumps(params, methodname=method,
                      allow_none=True).encode().hex(),
            f"write call {method}")
    for value in VALUES:
        add(["reply", ":" + describe(value).encode().hex()],
            rpc.dumps((value,), methodresponse=True,
                      allow_none=True).encode().hex(),
            f"write reply {value!r}"[:50])
    for code, why in ((1, "bad"), (-32601, "method not found"), (0, "")):
        add(["fault", ":" + ("%d|%s" % (code, why)).encode().hex()],
            rpc.dumps(rpc.Fault(code, why)).encode().hex(),
            f"write fault {code}")
    return argv, expected, labels
