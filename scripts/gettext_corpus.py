"""The catalogues and plural rules the gettext gate checks against.

Split out from the gate itself because building a `.mo` by hand is most of
the code and none of the comparison.
"""
import gettext
import io
import struct

def build_mo(entries, big=False):
    keys=[k.encode("utf-8") for k,_ in entries]
    vals=[v.encode("utf-8") for _,v in entries]
    n=len(entries)
    otab=28; ttab=otab+n*8; pool=ttab+n*8
    fmt=">I" if big else "<I"
    out=bytearray()
    out += struct.pack("<I", 0xde120495 if big else 0x950412de)
    for v in (0,n,otab,ttab,0,0): out += struct.pack(fmt,v)
    body=bytearray(); offs=[]
    for b in keys+vals:
        offs.append(pool+len(body)); body += b + b"\0"
    for i,b in enumerate(keys): out += struct.pack(fmt,len(b))+struct.pack(fmt,offs[i])
    for i,b in enumerate(vals): out += struct.pack(fmt,len(b))+struct.pack(fmt,offs[len(keys)+i])
    out += body
    return bytes(out)
HEAD_EN = ("Project-Id-Version: t\nContent-Type: text/plain; charset=UTF-8\n"
           "Language: en\nPlural-Forms: nplurals=2; plural=(n != 1);\n")
HEAD_PL = ("Content-Type: text/plain; charset=UTF-8\nLanguage: pl\n"
           "Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 "
           "&& (n%100<10 || n%100>=20) ? 1 : 2);\n")
HEAD_JA = ("Content-Type: text/plain; charset=UTF-8\nLanguage: ja\n"
           "Plural-Forms: nplurals=1; plural=0;\n")
HEAD_AR = ("Content-Type: text/plain; charset=UTF-8\nLanguage: ar\n"
           "Plural-Forms: nplurals=6; plural=(n==0 ? 0 : n==1 ? 1 : n==2 ? 2"
           " : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5);\n")
CATALOGUES=[
 ("english", [("", HEAD_EN), ("Hello", "Hello"), ("Cancel", "Cancel"),
              ("file\0files", "file\0files"), ("menu\x04Open", "Open"),
              ("button\x04Open", "Open it")], False),
 ("polish", [("", HEAD_PL), ("Hello", "Cześć"),
             ("file\0files", "plik\0pliki\0plików")], False),
 ("japanese", [("", HEAD_JA), ("Hello", "こんにちは"),
               ("file\0files", "ファイル")], False),
 ("arabic", [("", HEAD_AR),
             ("file\0files", "صفر\0واحد\0اثنان\0قليل\0كثير\0آخر")], False),
 ("big-endian", [("", HEAD_EN), ("Hello", "Hi")], True),
]
RULES = ["n != 1", "0",
         "(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2)",
         "(n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : "
         "n%100>=11 ? 4 : 5)",
         "n>1",
         "(n%10==1 && n%100!=11 ? 0 : n%10>=2 && (n%100<10 || "
         "n%100>=20) ? 1 : 2)",
         "(n==1) ? 0 : ((n==0 || (n%100 > 0 && n%100 < 20)) ? 1 : 2)",
         "n==1 ? 0 : n==2 ? 1 : 2",
         "(n > 1)"]
def build():
    args=[]; expect=[]; labels=[]
    for label, entries, big in CATALOGUES:
        raw=build_mo(entries, big)
        cat=gettext.GNUTranslations(io.BytesIO(raw))
        for key in ("Hello", "Cancel", "missing"):
            args += ["get", ":"+raw.hex(), ":"+key.encode().hex()]
            expect.append(cat.gettext(key).encode().hex())
            labels.append("%s get %r" % (label, key))
        for name in ("Language", "Content-Type", "Nope"):
            args += ["header", ":"+raw.hex(), ":"+name.encode().hex()]
            expect.append(cat._info.get(name.lower(), "-").encode().hex())
            labels.append("%s header %s" % (label, name))
        for n in (0,1,2,3,4,5,11,12,21,22,100,101,102,111):
            payload=("file\x01files\x01%d" % n).encode()
            args += ["plural", ":"+raw.hex(), ":"+payload.hex()]
            expect.append(cat.ngettext("file","files",n).encode().hex())
            labels.append("%s plural %d" % (label, n))
        for ctx, key in (("menu","Open"), ("button","Open"), ("none","Open")):
            payload=("%s\x01%s" % (ctx,key)).encode()
            args += ["context", ":"+raw.hex(), ":"+payload.hex()]
            expect.append(cat.pgettext(ctx, key).encode().hex())
            labels.append("%s context %s/%s" % (label, ctx, key))
    args += ["get", ":"+b"not a mo file at all".hex(), ":"+b"x".hex()]
    expect.append("error"); labels.append("refuse non-mo")
    for rule in RULES:
        chooser = gettext.c2py(rule)
        for n in range(0, 130):
            args += ["rule", ":"+rule.encode().hex(), str(n)]
            expect.append(str(chooser(n)))
            labels.append("rule %r n=%d" % (rule[:24], n))
    return args, expect, labels
