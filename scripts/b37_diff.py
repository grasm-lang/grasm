"""Eleven image formats against Pillow.

Images are random in four styles (noise, runs, gradients, a handful of
colours) so that every coding path is used: QOI's six chunk kinds, RLE
runs and literals, palettes. For each format:

- files Pillow writes (in every mode and compression it offers) are decoded
  here and must give Pillow's own pixels;
- files written here are decoded by Pillow and must give back the image;
- where Pillow reads variants it does not write (16-bit and right-to-left
  TGA, 1-bit and colour-mapped Sun rasters, legacy RLE icons, XPM, plain
  PNM with comments), small Python writers make them and both decode;
- QOI's encoder must match Pillow's byte for byte, since QOI's encoding is
  fully specified.

A file Pillow refuses must be refused here too. Where Pillow reads a file
wrongly or refuses what the format allows, those inputs are left out and
the package says what it does instead: 24-bit PCX of odd width (Pillow
unpacks the planes by width, not bytes-per-line), Sun RLE rows that need
padding (Pillow drops it), TGA run packets crossing rows (Pillow refuses
them), XPM colours other than #RRGGBB, and APNG "over" blending with
partial alpha (Pillow pastes rather than composites).
"""
import io
import os
import random
import struct
import subprocess
import sys
import tempfile
import warnings

warnings.simplefilter("ignore")
from PIL import Image  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
rng = random.Random(20261019)
work = tempfile.mkdtemp(prefix="b37")
harness = os.path.join(work, "b37test")
built = subprocess.run([GRASM, "build", "packages/b37test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

CH = {"1": 1, "L": 1, "LA": 2, "RGB": 3, "RGBA": 4}


def raw_pixels(channels, w, h, style):
    n = w * h
    if style == "noise":
        return bytes(rng.randrange(256) for _ in range(n * channels))
    if style == "few":
        colours = [bytes(rng.randrange(256) for _ in range(channels)) for _ in range(rng.randrange(1, 6))]
        return b"".join(rng.choice(colours) for _ in range(n))
    if style == "runs":
        out, c = b"", bytes(rng.randrange(256) for _ in range(channels))
        while len(out) < n * channels:
            if rng.random() < 0.2:
                c = bytes(rng.randrange(256) for _ in range(channels))
            out += c * rng.randrange(1, 80)
        return out[:n * channels]
    out = bytearray()
    for y in range(h):
        for x in range(w):
            for c in range(channels):
                out.append((x * 7 + y * 3 + c * 50 + rng.randrange(3)) % 256)
    return bytes(out)


def image(mode, w=None, h=None):
    w = w or rng.randrange(1, 40)
    h = h or rng.randrange(1, 30)
    style = rng.choice(["noise", "few", "runs", "gradient"])
    if mode == "1":
        return Image.frombytes("L", (w, h), raw_pixels(1, w, h, style)).point(lambda v: 255 if v > 127 else 0).convert("1")
    if mode == "P":
        img = Image.frombytes("RGB", (w, h), raw_pixels(3, w, h, "few"))
        return img.convert("P", palette=Image.Palette.ADAPTIVE, colors=rng.choice([2, 16, 200]))
    return Image.frombytes(mode, (w, h), raw_pixels(CH[mode], w, h, style))


def save(img, fmt, **kw):
    b = io.BytesIO()
    img.save(b, fmt, **kw)
    return b.getvalue()


def pillow(data, convert=None, size=None):
    try:
        im = Image.open(io.BytesIO(data))
        if size:
            # Pillow 12 takes the size as a pair and the scale at load.
            im.size = size[:2]
            im.load(scale=size[2])
        im.load()
        return im.convert(convert) if convert else im
    except Exception:
        return None


cases, wants = [], []


def add(op, fields, want, family=None):
    cases.append((op, "|".join(str(f) for f in fields), family or op))
    wants.append(want)


def summary(*parts):
    return "|".join(str(p) for p in parts)


# --- QOI -------------------------------------------------------------------------------------------
for _ in range(150):
    mode = rng.choice(["RGB", "RGBA"])
    img = image(mode)
    data = save(img, "QOI")
    w, h = img.size
    add("qoi.dec", [data.hex()], summary(w, h, CH[mode], data[13], img.tobytes().hex()))
    add("qoi.enc", [img.tobytes().hex(), w, h, CH[mode], data[13]], data.hex(), "qoi.encbytes")
    if rng.random() < 0.3:
        cut = data[:rng.randrange(14, len(data))]
        add("qoi.dec", [cut.hex()], "ERR" if pillow(cut) is None else "skip")

# --- PNM ------------------------------------------------------------------------------------------
def pnm_want(data):
    im = pillow(data)
    if im is None:
        return "ERR"
    w, h = im.size
    if im.mode == "1":
        return summary(w, h, 1, 8, im.convert("L").tobytes().hex())
    if im.mode in ("I", "I;16"):
        vals = im.get_flattened_data() if hasattr(im, "get_flattened_data") else im.getdata()
        return summary(w, h, 1, 16, b"".join(int(v).to_bytes(2, "big") for v in vals).hex())
    return summary(w, h, CH[im.mode], 8, im.tobytes().hex())


for _ in range(60):
    mode = rng.choice(["1", "L", "RGB", "I;16"])
    img = image("L") if mode == "I;16" else image(mode)
    if mode == "I;16":
        img = Image.frombytes("I;16", img.size, bytes(rng.randrange(256) for _ in range(2 * img.size[0] * img.size[1])))
    data = save(img, "PPM")
    add("pnm.dec", [data.hex()], pnm_want(data))


def pnm_text(kind, w, h, maxval, samples, comments):
    head = f"P{kind}\n"
    if comments:
        head += "# a comment\n"
    # A comment after the last header token would be raster data (one
    # whitespace byte ends the header), so comments stay on their own line.
    head += f"{w} {h}\n"
    if kind not in (1, 4):
        head += f"{maxval}\n"
    body = b""
    if kind in (1, 2, 3):
        sep = "" if kind == 1 and rng.random() < 0.5 else " "
        body = (sep.join(str(v) for v in samples) + "\n").encode()
    elif kind == 4:
        row = (w + 7) // 8
        out = bytearray()
        for y in range(h):
            line = bytearray(row)
            for x in range(w):
                if samples[y * w + x]:
                    line[x // 8] |= 1 << (7 - x % 8)
            out += line
        body = bytes(out)
    else:
        body = b"".join(v.to_bytes(2 if maxval > 255 else 1, "big") for v in samples)
    return head.encode() + body


for _ in range(120):
    kind = rng.randrange(1, 7)
    w, h = rng.randrange(1, 20), rng.randrange(1, 15)
    maxval = 1 if kind in (1, 4) else rng.choice([1, 7, 100, 255, 256, 1000, 65535])
    chans = 3 if kind in (3, 6) else 1
    samples = [rng.randrange(maxval + 1) for _ in range(w * h * chans)]
    data = pnm_text(kind, w, h, maxval, samples, rng.random() < 0.5)
    add("pnm.dec", [data.hex()], pnm_want(data))
    if kind in (4, 5, 6) or kind in (1, 2, 3):
        op = "pnm.enc" if kind >= 4 else "pnm.plain"
        wide = maxval > 255 and kind not in (1, 4)
        hexed = b"".join(v.to_bytes(2 if wide else 1, "big") for v in samples).hex()
        add(op, [hexed, w, h, chans, maxval, 1 if kind in (1, 4) else 0], ("pnm-same", data), "pnm.encoded")

# --- TGA -------------------------------------------------------------------------------------------
def tga_want(data):
    im = pillow(data)
    if im is None:
        return "ERR"
    if im.mode == "P":
        has_alpha = im.palette.mode == "RGBA"
        im = im.convert("RGBA" if has_alpha else "RGB")
    return summary(im.size[0], im.size[1], CH[im.mode], im.tobytes().hex())


for _ in range(100):
    mode = rng.choice(["L", "LA", "RGB", "RGBA", "P"])
    img = image(mode)
    data = save(img, "TGA", compression=rng.choice([None, "tga_rle"]))
    add("tga.dec", [data.hex()], tga_want(data))
for _ in range(80):
    # 16-bit and odd orientations, written here.
    w, h = rng.randrange(1, 20), rng.randrange(1, 15)
    alpha = rng.random() < 0.5
    desc = (1 if alpha else 0) | rng.choice([0, 0x10, 0x20, 0x30])
    rle = rng.random() < 0.5
    px = [rng.choice([rng.randrange(65536), 0x7fff, 0x8000]) for _ in range(w * h)]
    body = b""
    if rle:
        # Run packets stay within a row: TGA 2.0 says writers should keep
        # them so, and Pillow refuses one that crosses (tga accepts it).
        i = 0
        while i < len(px):
            run = 1
            while i + run < len(px) and run < 128 and px[i + run] == px[i] and (i + run) % w != 0:
                run += 1
            if run > 1:
                body += bytes([127 + run]) + struct.pack("<H", px[i])
            else:
                body += bytes([0]) + struct.pack("<H", px[i])
            i += run
    else:
        body = b"".join(struct.pack("<H", v) for v in px)
    data = bytes([0, 0, 10 if rle else 2]) + bytes(9) + struct.pack("<HHBB", w, h, 16, desc) + body
    add("tga.dec", [data.hex()], tga_want(data), "tga.dec16")
for _ in range(60):
    mode = rng.choice(["L", "LA", "RGB", "RGBA"])
    img = image(mode)
    add("tga.enc", [img.tobytes().hex(), img.size[0], img.size[1], CH[mode], rng.choice([0, 1])], ("pillow-same", mode, img.tobytes()), "tga.encoded")

# --- PCX -------------------------------------------------------------------------------------------
for _ in range(80):
    mode = rng.choice(["1", "L", "P", "RGB"])
    # Pillow misreads 24-bit PCX of odd width, its own included: it unpacks
    # the planes by the width, not by the padded bytes-per-line. pcx reads
    # them by bytes-per-line, as the format says; only even widths are
    # compared.
    img = image(mode, rng.randrange(1, 20) * 2 if mode == "RGB" else None)
    data = save(img, "PCX")
    im = pillow(data, "RGB")
    add("pcx.dec", [data.hex()], "ERR" if im is None else summary(im.size[0], im.size[1], im.tobytes().hex()))
for _ in range(40):
    img = image("RGB", rng.randrange(1, 20) * 2)
    add("pcx.rgb", [img.tobytes().hex(), img.size[0], img.size[1]], ("pillow-same", "RGB", img.tobytes()), "pcx.encoded")
    pimg = image("P")
    pal = bytes(pimg.getpalette()[:768])
    add("pcx.pal", [pimg.tobytes().hex(), pimg.size[0], pimg.size[1], pal.hex()], ("pillow-same", "RGB", pimg.convert("RGB").tobytes()), "pcx.encoded")

# --- XBM -------------------------------------------------------------------------------------------
for _ in range(60):
    img = image("1")
    data = save(img, "XBM")
    im = pillow(data, "L")
    add("xbm.dec", [data.hex()], summary(im.size[0], im.size[1], im.tobytes().hex()))
    px = img.convert("L").tobytes()
    add("xbm.enc", [px.hex(), img.size[0], img.size[1], "img"], ("pillow-same", "L", px), "xbm.encoded")

# --- XPM -------------------------------------------------------------------------------------------
for _ in range(80):
    w, h = rng.randrange(1, 16), rng.randrange(1, 12)
    cpp = rng.choice([1, 1, 2])
    ncol = rng.randrange(1, 8)
    alphabet = ".#abcdefgh+@"
    keys = []
    while len(keys) < ncol:
        k = "".join(rng.choice(alphabet) for _ in range(cpp))
        if k not in keys:
            keys.append(k)
    lines = [f"{w} {h} {ncol} {cpp}"]
    for k in keys:
        # Pillow reads only #RRGGBB: it refuses colour names and crashes on
        # pixels of a None colour, so those are not checked against it.
        c = "#%02X%02X%02X" % (rng.randrange(256), rng.randrange(256), rng.randrange(256))
        lines.append(f"{k} c {c}")
    for y in range(h):
        lines.append("".join(rng.choice(keys) for _ in range(w)))
    text = "/* XPM */\nstatic char *img[] = {\n" + ",\n".join('"' + l + '"' for l in lines) + "};\n"
    data = text.encode()
    im = pillow(data, "RGBA")
    add("xpm.dec", [data.hex()], "ERR" if im is None else summary(im.size[0], im.size[1], im.tobytes().hex()))

# --- SGI -------------------------------------------------------------------------------------------
for _ in range(60):
    mode = rng.choice(["L", "RGB", "RGBA"])
    img = image(mode)
    data = save(img, "SGI")
    im = pillow(data)
    add("sgi.dec", [data.hex()], "ERR" if im is None else summary(im.size[0], im.size[1], CH[im.mode], 1, im.tobytes().hex()))
for _ in range(60):
    ch, bpc = rng.choice([1, 3, 4]), rng.choice([1, 1, 2])
    w, h = rng.randrange(1, 20), rng.randrange(1, 15)
    vals = [rng.choice([rng.randrange(256 ** bpc), 0, 0, 7]) for _ in range(w * h * ch)]
    hexed = b"".join(v.to_bytes(bpc, "big") for v in vals).hex()
    add("sgi.enc", [hexed, w, h, ch, bpc, rng.choice([0, 1])], ("sgi-roundtrip", w, h, ch, bpc, hexed), "sgi.encoded")

# --- DDS -------------------------------------------------------------------------------------------
for _ in range(90):
    choice = rng.choice([("RGB", {}), ("RGBA", {}), ("RGBA", {"pixel_format": "DXT1"}), ("RGBA", {"pixel_format": "DXT3"}),
                         ("RGBA", {"pixel_format": "DXT5"}), ("RGB", {"pixel_format": "BC5"}), ("L", {"pixel_format": "BC4"}),
                         ("L", {}), ("LA", {})])
    img = image(choice[0])
    try:
        data = save(img, "DDS", **choice[1])
    except Exception:
        continue
    im = pillow(data)
    add("dds.dec", [data.hex()], "ERR" if im is None else summary(im.size[0], im.size[1], CH.get(im.mode, 0), im.tobytes().hex()),
        "dds." + choice[1].get("pixel_format", choice[0]))

# --- ICNS ------------------------------------------------------------------------------------------
for _ in range(3):
    # A smooth icon: noise scaled to 1024 pixels would not compress, and
    # the files go to the harness as arguments.
    grad = bytes((x * 4 + c * 60 + y) % 256 if c < 3 else 255 - (x + y) % 256 for y in range(64) for x in range(64) for c in range(4))
    base = Image.frombytes("RGBA", (64, 64), grad).resize((128, 128))
    data = save(base, "ICNS")
    im = Image.open(io.BytesIO(data))
    for size in sorted(im.info.get("sizes", [])):
        s, scale = size[0], size[2]
        if s * scale > 256:
            continue
        got = pillow(data, "RGBA", size=size)
        add("icns.dec", [data.hex(), s, scale], summary(got.size[0], got.size[1], got.tobytes().hex()), "icns.png")


def icns_rle(plane):
    out, i = b"", 0
    while i < len(plane):
        run = 1
        while i + run < len(plane) and run < 130 and plane[i + run] == plane[i]:
            run += 1
        if run >= 3:
            out += bytes([run + 125, plane[i]])
            i += run
        else:
            lit = 1
            while i + lit < len(plane) and lit < 128 and not (i + lit + 2 < len(plane) and plane[i + lit] == plane[i + lit + 1] == plane[i + lit + 2]):
                lit += 1
            out += bytes([lit - 1]) + bytes(plane[i:i + lit])
            i += lit
    return out


for _ in range(20):
    entries = b""
    sizes = rng.sample([(16, "is32", "s8mk"), (32, "il32", "l8mk"), (48, "ih32", "h8mk"), (128, "it32", "t8mk")], rng.randrange(1, 3))
    for s, kind, mk in sizes:
        img = image("RGBA", s, s)
        px = img.tobytes()
        body = (b"\0\0\0\0" if kind == "it32" else b"") + b"".join(icns_rle(px[c::4]) for c in range(3))
        entries += kind.encode() + struct.pack(">I", 8 + len(body)) + body
        entries += mk.encode() + struct.pack(">I", 8 + s * s) + px[3::4]
    data = b"icns" + struct.pack(">I", 8 + len(entries)) + entries
    for s, kind, mk in sizes:
        got = pillow(data, "RGBA", size=(s, s, 1))
        add("icns.dec", [data.hex(), s, 1], "ERR" if got is None else summary(s, s, got.tobytes().hex()), "icns.legacy")

# --- Sun raster ------------------------------------------------------------------------------------
for _ in range(60):
    mode = rng.choice(["L", "RGB"])
    rle = rng.choice([0, 1])
    # Pillow decodes RLE without the rows' 16-bit padding, so RLE cases
    # use even widths, which need none.
    w = rng.randrange(1, 20) * (2 if rle else 1)
    img = image(mode, w)
    add("sun.enc", [img.tobytes().hex(), img.size[0], img.size[1], CH[mode], rle], ("pillow-same", mode, img.tobytes()), "sun.encoded")
for _ in range(80):
    w, h = rng.randrange(1, 20), rng.randrange(1, 15)
    depth = rng.choice([1, 8, 8, 24, 32])
    kind = rng.choice([1, 2, 3]) if depth >= 24 else rng.choice([1, 2])
    if kind == 2:
        # Rows that need no padding (see sun.enc above).
        w = rng.randrange(1, 3) * 16 if depth == 1 else rng.randrange(1, 10) * 2
    cmap = b""
    if depth == 8 and rng.random() < 0.5:
        n = rng.randrange(2, 256)
        cmap = bytes(rng.randrange(256) for _ in range(3 * n))
        colours = n
    else:
        colours = 256
    row = ((w * depth + 15) // 16) * 2
    raw = bytearray()
    for y in range(h):
        line = bytearray(rng.randrange(256) if depth != 8 else rng.randrange(colours) for _ in range(row))
        if rng.random() < 0.5:
            line = bytearray([line[0]] * row)
        raw += line
    if rng.random() < 0.2:
        raw = bytearray(b"\x80" * len(raw))
    body = bytes(raw)
    if kind == 2:
        out, i = b"", 0
        while i < len(raw):
            run = 1
            while i + run < len(raw) and run < 256 and raw[i + run] == raw[i]:
                run += 1
            if run >= 3 or raw[i] == 0x80:
                out += b"\x80\x00" if (raw[i] == 0x80 and run == 1) else bytes([0x80, run - 1, raw[i]])
            else:
                out += bytes(raw[i:i + run])
            i += run
        body = out
    data = struct.pack(">8I", 0x59A66A95, w, h, depth, len(body), kind, 1 if cmap else 0, len(cmap)) + cmap + body
    im = pillow(data)
    if im is not None and im.mode == "P":
        im = im.convert("RGB")
    if im is not None and im.mode == "1":
        im = im.convert("L")
    add("sun.dec", [data.hex()], "ERR" if im is None else summary(im.size[0], im.size[1], CH.get(im.mode, 0), im.tobytes().hex()), f"sun.dec{depth}")

# --- APNG ------------------------------------------------------------------------------------------
for _ in range(40):
    w, h = rng.randrange(2, 24), rng.randrange(2, 18)
    n = rng.randrange(2, 5)
    frames = []
    for _f in range(n):
        f = image("RGBA", w, h)
        # Blending over is checked only where alpha is 0 or 255, where
        # Pillow's paste agrees with the specification's "over".
        f = Image.frombytes("RGBA", (w, h), bytes(v if i % 4 != 3 else (255 if v > 127 else 0) for i, v in enumerate(f.tobytes())))
        frames.append(f)
    durations = [rng.choice([10, 40, 100, 1000]) for _ in range(n)]
    disposal = [rng.randrange(3) for _ in range(n)]
    blend = [rng.randrange(2) for _ in range(n)]
    loop = rng.choice([0, 1, 3])
    b = io.BytesIO()
    frames[0].save(b, "PNG", save_all=True, append_images=frames[1:], duration=durations, disposal=disposal, blend=blend, loop=loop)
    data = b.getvalue()
    im = Image.open(io.BytesIO(data))
    got = []
    for i in range(im.n_frames):
        im.seek(i)
        got.append((durations[i], im.convert("RGBA").tobytes().hex()))
    add("apng.dec", [data.hex()], ("apng", w, h, loop, got))

# --- run and compare -------------------------------------------------------------------------------
answers = []


def arg_of(fields):
    """Inline hex, or a file for a case too large for an argument."""
    h = fields.encode().hex()
    if len(h) < 200000:
        return "x" + h
    path = os.path.join(work, f"case{len(os.listdir(work))}.hex")
    open(path, "w").write(h)
    return "@" + path


def flush(batch):
    argv = []
    for op, fields, _ in batch:
        argv += [op, "x", arg_of(fields)]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=1800)
    if ran.returncode != 0:
        for op, fields, _ in batch:
            one = subprocess.run([harness, op, "x", arg_of(fields)], capture_output=True, text=True)
            if one.returncode != 0:
                print(f"harness crashed on {op} {fields[:200]}")
                break
        print(ran.stderr[-2000:]); sys.exit(1)
    answers.extend(ran.stdout.split("\n")[:-1])


batch, size = [], 0
for c in cases:
    if batch and (size + min(2 * len(c[1]), 200000) > 400000 or len(batch) >= 60):
        flush(batch)
        batch, size = [], 0
    batch.append(c)
    size += min(2 * len(c[1]), 200000) + 30
if batch:
    flush(batch)

families, differ = {}, 0
only = os.environ.get("ONLY", "")
for (op, fields, fam), want, got in zip(cases, wants, answers):
    mine = "ERR" if got == "ERR" else bytes.fromhex(got).decode()
    if isinstance(want, tuple):
        kind = want[0]
        if mine == "ERR":
            ok = False
        elif kind == "apng":
            # Delays are compared as values: 10 ms may be written 1/100.
            _, w, h, loop, frames = want
            parts = mine.split("|")
            fr = parts[3].split(",") if len(parts) > 3 else []
            ok = parts[:3] == [str(w), str(h), str(loop)] and len(fr) == len(frames)
            for f, (ms, px) in zip(fr, frames) if ok else []:
                d, hexed = f.split(":")
                num, den = (int(x) for x in d.split("/"))
                ok = ok and num * 1000 == ms * den and hexed == px
        elif kind == "pnm-same":
            ok = pnm_want(bytes.fromhex(mine)) == pnm_want(want[1])
        elif kind == "pillow-same":
            im = pillow(bytes.fromhex(mine), want[1])
            ok = im is not None and im.tobytes() == want[2]
        else:
            ok = mine != "ERR"
            if ok:
                _, w, h, ch, bpc, hexed = want
                im = pillow(bytes.fromhex(mine))
                ok = im is not None and im.size == (w, h)
                if ok and bpc == 1:
                    ok = im.tobytes().hex() == hexed
        mine, want = ("ok" if ok else mine[:60]), "ok"
    if want == "skip":
        continue
    f = families.setdefault(fam, [0, 0])
    if mine == want:
        f[0] += 1
    else:
        f[1] += 1
        differ += 1
        if fam.startswith(only) and (differ <= 8 or (only and f[1] <= 8)):
            print(f"  {fam} {op} {fields[:100]}")
            print(f"    want  {str(want)[:200]}")
            print(f"    grasm {mine[:200]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
print(f"families: {len(families)}  " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
thin = [k for k, (a, d) in families.items() if a + d < 5]
if thin or len(families) < 25:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {sum(a + d for a, d in families.values())}   agreeing: {sum(a for a, d in families.values())}   differing: {differ}")
sys.exit(1 if differ else 0)
