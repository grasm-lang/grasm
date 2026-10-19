"""OpenType tables against fontTools.

Every font macOS ships in /System/Library/Fonts (collections opened font
by font), plus fonts built here with fontTools' FontBuilder to reach what
system fonts lack (COLR and CPAL in both versions, avar, STAT in every
axis-value format, fvar with and without PostScript name IDs). For each
font, each of `name`, `OS/2`, `post`, `kern`, `fvar`, `avar`, `STAT`,
`COLR`, `CPAL` and `GSUB` it has is handed to the harness as its raw bytes,
and the summary must equal one built from fontTools' parsed table (glyph
names mapped back to glyph IDs through the font's glyph order). Each
collection's directory is compared with fontTools' TTC reader.
"""
import glob
import io
import os
import subprocess
import sys
import tempfile
import warnings

warnings.simplefilter("ignore")
import logging  # noqa: E402

logging.disable(logging.CRITICAL)
from fontTools.ttLib import TTFont, TTCollection  # noqa: E402
from fontTools.ttLib.sfnt import readTTCHeader, SFNTReader  # noqa: E402
from fontTools.misc.fixedTools import floatToFixed  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
work = tempfile.mkdtemp(prefix="b38")
harness = os.path.join(work, "b38test")
built = subprocess.run([GRASM, "build", "packages/b38test/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

cases = []   # (op, path, want, family)
counter = [0]


def table_file(data):
    counter[0] += 1
    p = os.path.join(work, f"t{counter[0]}")
    with open(p, "wb") as f:
        f.write(data)
    return p


def ints(v):
    return ",".join(str(x) for x in v)


def gid(font, name):
    return font.getGlyphID(name)


def want_name(font):
    t = font["name"]
    parts = []
    for r in t.names:
        raw = r.string if isinstance(r.string, bytes) else r.toBytes()
        decoded = r.platformID in (0, 3) or (r.platformID == 1 and r.platEncID == 0)
        if decoded:
            text = raw.decode("utf_16_be", "replace") if r.platformID in (0, 3) else raw.decode("mac_roman")
            body = "t" + text.encode("utf-8", "surrogatepass").hex()
        else:
            body = "x" + raw.hex()
        parts.append(f"{r.platformID},{r.platEncID},{r.langID},{r.nameID}:{body}")
    raw = font.reader["name"]
    return f"{int.from_bytes(raw[0:2], 'big')}|" + ";".join(sorted(parts))


def want_os2(font):
    t = font["OS/2"]
    p = t.panose
    a = [t.version, t.xAvgCharWidth, t.usWeightClass, t.usWidthClass, t.fsType, t.ySubscriptXSize, t.ySubscriptYSize,
         t.ySubscriptXOffset, t.ySubscriptYOffset, t.ySuperscriptXSize, t.ySuperscriptYSize, t.ySuperscriptXOffset,
         t.ySuperscriptYOffset, t.yStrikeoutSize, t.yStrikeoutPosition, t.sFamilyClass, p.bFamilyType, p.bSerifStyle, p.bWeight,
         p.bProportion, p.bContrast, p.bStrokeVariation, p.bArmStyle, p.bLetterForm, p.bMidline, p.bXHeight, t.ulUnicodeRange1,
         t.ulUnicodeRange2, t.ulUnicodeRange3, t.ulUnicodeRange4, t.fsSelection, t.usFirstCharIndex, t.usLastCharIndex,
         t.sTypoAscender, t.sTypoDescender, t.sTypoLineGap, t.usWinAscent, t.usWinDescent]
    if t.version >= 1:
        a += [t.ulCodePageRange1, t.ulCodePageRange2]
    if t.version >= 2:
        a += [t.sxHeight, t.sCapHeight, t.usDefaultChar, t.usBreakChar, t.usMaxContext]
    if t.version >= 5:
        a += [t.usLowerOpticalPointSize, t.usUpperOpticalPointSize]
    return ints(a) + "|" + font.reader["OS/2"][58:62].hex()


def check_post(font, mine):
    t = font["post"]
    raw = font.reader["post"]
    f = mine.split("|", 1)
    nums = [int(x) for x in f[0].split(",")]
    ok = nums[0] == int.from_bytes(raw[0:4], "big") and abs(nums[1] / 65536 - t.italicAngle) < 1e-4
    ok = ok and nums[2:9] == [t.underlinePosition, t.underlineThickness, t.isFixedPitch, t.minMemType42, t.maxMemType42,
                              t.minMemType1, t.maxMemType1]
    names = f[1].split(" ") if f[1] else []
    if nums[0] == 0x20000:
        # The post table's own glyph order (a CFF font's comes from its
        # charset instead). fontTools renames repeated names (".notdef.1"),
        # so the names are compared only where they are all different.
        order = getattr(t, "glyphOrder", None) or font.getGlyphOrder()
        if len(set(names)) == len(names):
            ok = ok and names == order
        ok = ok and len(names) == len(order)
    return ok


def want_kern(font):
    out = []
    for st in font["kern"].kernTables:
        pairs = []
        if st.format == 0:
            pairs = sorted(f"{gid(font, l)},{gid(font, r)},{v}" for (l, r), v in st.kernTable.items())
        out.append(f"{st.format}:{st.coverage}:" + ";".join(pairs))
    return "|".join(out)


def want_fvar(font):
    t = font["fvar"]
    axes = ";".join(f"{a.axisTag},{floatToFixed(a.minValue, 16)},{floatToFixed(a.defaultValue, 16)},{floatToFixed(a.maxValue, 16)},"
                    f"{a.flags},{a.axisNameID}" for a in t.axes)
    inst = ";".join(f"{i.subfamilyNameID},{i.flags}," + ints(floatToFixed(i.coordinates[a.axisTag], 16) for a in t.axes) +
                    f",{i.postscriptNameID}" for i in t.instances)
    return axes + "|" + inst


def want_avar(font):
    seg = font["avar"].segments
    return ";".join(",".join(f"{floatToFixed(k, 14)}:{floatToFixed(v, 14)}" for k, v in seg[a.axisTag].items()) for a in font["fvar"].axes)


def want_stat(font):
    t = font["STAT"].table
    axes = ";".join(f"{a.AxisTag},{a.AxisNameID},{a.AxisOrdering}" for a in (t.DesignAxisRecord.Axis if t.DesignAxisRecord else []))
    vals = []
    for v in (t.AxisValueArray.AxisValue if t.AxisValueArray else []):
        if v.Format == 4:
            pairs = "+".join(f"{r.AxisIndex}:{floatToFixed(r.Value, 16)}" for r in v.AxisValueRecord)
            extra = []
        else:
            base = v.NominalValue if v.Format == 2 else v.Value
            pairs = f"{v.AxisIndex}:{floatToFixed(base, 16)}"
            extra = [v.RangeMinValue, v.RangeMaxValue] if v.Format == 2 else [v.LinkedValue] if v.Format == 3 else []
        vals.append(f"{v.Format},{v.Flags},{v.ValueNameID},{pairs}," + ints(floatToFixed(x, 16) for x in extra))
    minor = t.Version & 0xFFFF
    elided = getattr(t, "ElidedFallbackNameID", 2) if minor >= 1 else 2
    return f"{minor}|{axes}|" + ";".join(vals) + f"|{elided}"


def want_colr(font):
    c = font["COLR"]
    if c.version == 0:
        return ";".join(f"{gid(font, b)}:" + "+".join(f"{gid(font, l.name)},{l.colorID}" for l in ls) for b, ls in c.ColorLayers.items())
    t = c.table
    recs = t.BaseGlyphRecordArray.BaseGlyphRecord if t.BaseGlyphRecordArray else []
    layers = t.LayerRecordArray.LayerRecord if t.LayerRecordArray else []
    return ";".join(f"{gid(font, r.BaseGlyph)}:" + "+".join(f"{gid(font, l.LayerGlyph)},{l.PaletteIndex}"
                                                             for l in layers[r.FirstLayerIndex:r.FirstLayerIndex + r.NumLayers]) for r in recs)


def want_cpal(font):
    c = font["CPAL"]
    types = getattr(c, "paletteTypes", None) or [0] * len(c.palettes)
    labels = getattr(c, "paletteLabels", None) or [0xFFFF] * len(c.palettes)
    if c.version == 0:
        types, labels = [0] * len(c.palettes), [0xFFFF] * len(c.palettes)
    return ";".join("+".join(f"{x.red},{x.green},{x.blue},{x.alpha}" for x in p) + f"/{types[i]}/{labels[i]}" for i, p in enumerate(c.palettes))


def want_gsub(font):
    t = font["GSUB"].table
    parts = []
    for sr in t.ScriptList.ScriptRecord:
        s = sr.Script
        if s.DefaultLangSys:
            parts.append(f"S {sr.ScriptTag} dflt {s.DefaultLangSys.ReqFeatureIndex} " + ints(s.DefaultLangSys.FeatureIndex))
        for lr in s.LangSysRecord:
            parts.append(f"S {sr.ScriptTag} {lr.LangSysTag} {lr.LangSys.ReqFeatureIndex} " + ints(lr.LangSys.FeatureIndex))
    for i, fr in enumerate(t.FeatureList.FeatureRecord):
        parts.append(f"F {i} {fr.FeatureTag} " + ints(fr.Feature.LookupListIndex))
    for i, lk in enumerate(t.LookupList.Lookup):
        subs = [st.ExtSubTable if lk.LookupType == 7 else st for st in lk.SubTable]
        kind = subs[0].LookupType if lk.LookupType == 7 and subs else lk.LookupType
        es = []
        for st in subs:
            k = st.LookupType
            if k == 1:
                es += [f"{gid(font, a)}>{gid(font, b)}" for a, b in st.mapping.items()]
            elif k in (2, 3):
                m = st.mapping if k == 2 else st.alternates
                es += [f"{gid(font, a)}>" + ints(gid(font, x) for x in b) for a, b in m.items()]
            elif k == 4:
                for first, ligs in st.ligatures.items():
                    for lg in ligs:
                        es.append(ints([gid(font, first)] + [gid(font, c) for c in lg.Component]) + f">{gid(font, lg.LigGlyph)}")
        mark = lk.MarkFilteringSet if lk.LookupFlag & 16 else -1
        parts.append(f"L {i} {kind} {lk.LookupFlag} {mark} " + ";".join(sorted(es)))
    return "\n".join(parts)


WANT = {"name": want_name, "OS/2": want_os2, "kern": want_kern, "fvar": want_fvar, "avar": want_avar, "STAT": want_stat,
        "COLR": want_colr, "CPAL": want_cpal, "GSUB": want_gsub}
OPS = {"name": "name", "OS/2": "os2", "post": "post", "kern": "kern", "fvar": "fvar", "avar": "avar", "STAT": "stat",
       "COLR": "colr", "CPAL": "cpal", "GSUB": "gsub"}


def add_font(font, label):
    for tag in OPS:
        if tag not in font.reader:
            continue
        if tag == "avar" and ("fvar" not in font.reader or font.reader["avar"][0:2] != b"\x00\x01"):
            continue
        try:
            raw = font.reader[tag]
            want = ("post", font) if tag == "post" else WANT[tag](font)
        except Exception:
            continue
        cases.append((OPS[tag], table_file(raw), want, OPS[tag]))


paths = sorted(glob.glob("/System/Library/Fonts/*.tt[fc]") + glob.glob("/System/Library/Fonts/*.otf") +
               glob.glob("/System/Library/Fonts/Supplemental/*.tt[fc]") + glob.glob("/System/Library/Fonts/Supplemental/*.otf"))
for path in paths[:200]:
    try:
        if path.endswith(".ttc"):
            with open(path, "rb") as f:
                head = readTTCHeader(f)
                want = f"{head.Version}|"
                fonts = []
                for i in range(head.numFonts):
                    r = SFNTReader(f, fontNumber=i)
                    fonts.append(f"{int.from_bytes(r.sfntVersion if isinstance(r.sfntVersion, bytes) else r.sfntVersion.encode('latin-1'), 'big')}:" +
                                 ";".join(f"{tag},{e.offset},{e.length}" for tag, e in r.tables.items()))
            cases.append(("ttc", path, want + "#".join(fonts), "ttc"))
            for font in TTCollection(path, lazy=True).fonts:
                add_font(font, path)
        else:
            add_font(TTFont(path, lazy=True), path)
    except Exception as e:
        continue

# --- fonts built here --------------------------------------------------------------------------
from fontTools.fontBuilder import FontBuilder  # noqa: E402
from fontTools.pens.ttGlyphPen import TTGlyphPen  # noqa: E402
import random  # noqa: E402

rng = random.Random(20261019)


def box():
    pen = TTGlyphPen(None)
    pen.moveTo((0, 0)); pen.lineTo((0, 500)); pen.lineTo((500, 500)); pen.lineTo((500, 0)); pen.closePath()
    return pen.glyph()


for n in range(40):
    glyphs = [".notdef"] + [f"g{i}" for i in range(rng.randrange(4, 20))]
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(glyphs)
    fb.setupCharacterMap({65 + i: g for i, g in enumerate(glyphs[1:])})
    fb.setupGlyf({g: box() for g in glyphs})
    fb.setupHorizontalMetrics({g: (500, 0) for g in glyphs})
    fb.setupHorizontalHeader()
    fb.setupNameTable({"familyName": "Grasm Test", "styleName": "Regular"})
    fb.setupOS2()
    fb.setupPost()
    axes = []
    if rng.random() < 0.7:
        tags = rng.sample(["wght", "wdth", "opsz", "slnt", "XOPQ"], rng.randrange(1, 4))
        axes = [(t, rng.choice([-20, 0, 8, 100]), 0, 0) for t in tags]
        axes = [(t, lo, lo + rng.randrange(1, 300) / 4, lo + rng.randrange(300, 900) / 4) for t, lo, _, _ in axes]
        fb.setupFvar(axes=[(t, lo, d, hi, t.upper()) for t, lo, d, hi in axes],
                     instances=[dict(location={t: rng.choice([lo, d, hi]) for t, lo, d, hi in axes}, stylename=f"S{k}",
                                     postscriptfontname=(f"Grasm-S{k}" if rng.random() < 0.5 else None)) for k in range(rng.randrange(0, 4))])
        if rng.random() < 0.7:
            from fontTools.ttLib import newTable
            av = newTable("avar")
            av.segments = {t: {-1.0: -1.0, 0.0: 0.0, rng.choice([0.25, 0.5, 0.75]): rng.choice([0.125, 0.375, 0.8125]), 1.0: 1.0}
                           for t, *_ in axes}
            fb.font["avar"] = av
        if rng.random() < 0.8:
            stat_axes = [dict(tag=t, name=t, values=[dict(value=d, name="Regular", flags=2)] +
                              [dict(nominalValue=hi, rangeMinValue=d, rangeMaxValue=hi, name="Max")] +
                              ([dict(value=lo, linkedValue=hi, name="Lo")] if lo != hi else [])) for t, lo, d, hi in axes]
            locs = [dict(name="Combo", location={t: d for t, lo, d, hi in axes})] if len(axes) > 1 else []
            fb.setupStat(stat_axes, locations=locs, elidedFallbackName=rng.choice([2, "Regular"]))
    if rng.random() < 0.7:
        npal = rng.randrange(1, 4)
        nent = rng.randrange(1, 5)
        pals = [[(rng.random(), rng.random(), rng.random(), rng.choice([1.0, 0.5, 0])) for _ in range(nent)] for _ in range(npal)]
        types = [rng.randrange(0, 4) for _ in range(npal)] if rng.random() < 0.5 else None
        fb.setupCPAL(pals, paletteTypes=types)
        layers = {}
        for base in rng.sample(glyphs[1:], rng.randrange(1, 4)):
            layers[base] = [(rng.choice(glyphs[1:]), rng.choice(list(range(nent)) + [0xFFFF])) for _ in range(rng.randrange(1, 4))]
        if rng.random() < 0.5:
            # A version 1 table: one glyph drawn by a paint graph, the rest
            # still version 0 layers, which is what is read here.
            spare = [g for g in glyphs[1:] if g not in layers]
            if spare:
                layers[spare[0]] = {"Format": 10, "Glyph": glyphs[1], "Paint": {"Format": 2, "PaletteIndex": 0, "Alpha": 1.0}}
        fb.setupCOLR(layers)
    b = io.BytesIO()
    fb.save(b)
    try:
        add_font(TTFont(io.BytesIO(b.getvalue())), f"built{n}")
    except Exception as e:
        print("could not reload a built font:", e)

# --- normalisation and avar mapping, against fontTools.varLib.models -----------------------------
from fontTools.varLib.models import normalizeValue, piecewiseLinearMap  # noqa: E402
from fontTools.misc.roundTools import otRound  # noqa: E402

for _ in range(600):
    lo = rng.randrange(-1000, 1000) * 65536 // rng.choice([1, 4, 100])
    d = lo + rng.randrange(0, 1000) * 65536 // rng.choice([1, 4, 100])
    hi = d + rng.randrange(0, 1000) * 65536 // rng.choice([1, 4, 100])
    if rng.random() < 0.2:
        d = rng.choice([lo, hi])
    v = rng.choice([lo, d, hi, lo - 65536, hi + 65536, rng.randrange(lo - 65536, hi + 65537)])
    want = otRound(normalizeValue(v / 65536, (lo / 65536, d / 65536, hi / 65536)) * 16384)
    cases.append(("norm", f"{lo},{d},{hi},{v}", str(want), "fvar.normalise"))
for _ in range(600):
    xs = sorted(set([-16384, 0, 16384] + [rng.randrange(-16383, 16384) for _ in range(rng.randrange(0, 4))]))
    ys = sorted(rng.randrange(-16384, 16385) for _ in xs)
    ys[xs.index(-16384)], ys[xs.index(0)], ys[xs.index(16384)] = -16384, 0, 16384
    ys = [max(-16384, min(16384, y)) for y in ys]
    v = rng.choice(xs + [rng.randrange(-16384, 16385) for _ in range(3)])
    want = otRound(piecewiseLinearMap(v / 16384, {x / 16384: y / 16384 for x, y in zip(xs, ys)}) * 16384)
    cases.append(("amap", ",".join(f"{x}:{y}" for x, y in zip(xs, ys)) + f";{v}", str(want), "avar.apply"))

# --- run ---------------------------------------------------------------------------------------
answers = []
for i in range(0, len(cases), 200):
    argv = []
    for op, path, _, _ in cases[i:i + 200]:
        argv += [op, path]
    ran = subprocess.run([harness] + argv, capture_output=True, text=True, timeout=3600)
    if ran.returncode != 0:
        for op, path, _, _ in cases[i:i + 200]:
            one = subprocess.run([harness, op, path], capture_output=True, text=True)
            if one.returncode != 0:
                print(f"harness crashed on {op} {path}")
                break
        print(ran.stderr[-2000:]); sys.exit(1)
    answers.extend(ran.stdout.split("\n")[:-1])

families, differ = {}, 0
only = os.environ.get("ONLY", "")
for (op, path, want, fam), got in zip(cases, answers):
    mine = "ERR" if got in ("ERR", "NOFILE") else bytes.fromhex(got).decode()
    if isinstance(want, tuple):
        ok = mine != "ERR" and check_post(want[1], mine)
        mine, want = ("ok" if ok else mine[:100]), "ok"
    if op == "ttc" and mine != "ERR":
        # fontTools lists a font's tables by offset; compare them by tag.
        def by_tag(s):
            head, fonts = s.split("|", 1)
            return head + "|" + "#".join(f.split(":", 1)[0] + ":" + ";".join(sorted(f.split(":", 1)[1].split(";"))) for f in fonts.split("#"))
        mine, want = by_tag(mine), by_tag(want)
    f = families.setdefault(fam, [0, 0])
    if mine == want:
        f[0] += 1
    else:
        f[1] += 1
        differ += 1
        if fam.startswith(only) and (differ <= 8 or (only and f[1] <= 8)):
            print(f"  {fam} {path}")
            print(f"    want  {str(want)[:300]}")
            print(f"    grasm {mine[:300]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
print(f"families: " + " ".join(f"{k}={a + d}" for k, (a, d) in sorted(families.items())))
thin = [k for k, (a, d) in families.items() if a + d < 10]
if thin or len(families) < 11:
    print(f"too few cases to mean anything: {thin} ({len(families)} families)"); sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {len(cases) - differ}   differing: {differ}")
sys.exit(1 if differ else 0)
