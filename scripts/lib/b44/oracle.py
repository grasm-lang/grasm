"""The reference for b44_diff.py: fontTools, one font table at a time.

Usage: oracle.py <op> <font> <index> <dir> -- writes the raw tables the
harness reads into <dir> and prints what fontTools finds, in the
harness's line format.
"""
import os
import sys
from fontTools.ttLib import TTFont, TTCollection
from fontTools.pens.recordingPen import RecordingPen

op, path, idx, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
font = TTCollection(path).fonts[idx] if path.lower().endswith(".ttc") else TTFont(path)
gid = font.getGlyphID
order = font.getGlyphOrder()
q16 = lambda v: int(round(v * 65536))
f2 = lambda v: int(round(v * 16384))


def raw(tag, name=None):
    p = os.path.join(out, name or tag.strip().lower().replace("/", "_"))
    open(p, "wb").write(font.reader[tag])
    return p


def classes(cd):
    return ",".join(f"{g}:{k}" for g, k in sorted((gid(n), k) for n, k in cd.classDefs.items() if k)) if cd else ""


def value(r):
    return ",".join(str(getattr(r, k, 0) or 0) if r is not None else "0" for k in ("XPlacement", "YPlacement", "XAdvance", "YAdvance"))


def anchor(a):
    return "-" if a is None else f"{a.XCoordinate},{a.YCoordinate}"


def cff_lines(step):
    raw("CFF ", "cff")
    top = font["CFF "].cff.topDictIndex[0]
    lines = []
    for g in range(0, len(top.charset), step):
        name = top.charset[g]
        c = top.CharStrings[name]
        pen = RecordingPen()
        c.draw(pen)
        lines.append(f"{name} {q16(c.width)} " + " ".join(op_ + "".join(f"({q16(x)},{q16(y)})" for x, y in pts) for op_, pts in pen.value))
    return lines


def gpos_lines():
    raw("GPOS")
    t = font["GPOS"].table
    lines = []
    for sr in (t.ScriptList.ScriptRecord if t.ScriptList else []):
        if sr.Script.DefaultLangSys is not None:
            l = sr.Script.DefaultLangSys
            lines.append(f"langsys {sr.ScriptTag} dflt {l.ReqFeatureIndex} " + ",".join(map(str, l.FeatureIndex)))
        for lr in sr.Script.LangSysRecord:
            lines.append(f"langsys {sr.ScriptTag} {lr.LangSysTag} {lr.LangSys.ReqFeatureIndex} " + ",".join(map(str, lr.LangSys.FeatureIndex)))
    for fr in (t.FeatureList.FeatureRecord if t.FeatureList else []):
        lines.append(f"feature {fr.FeatureTag} " + ",".join(map(str, fr.Feature.LookupListIndex)))
    for i, lk in enumerate(t.LookupList.Lookup if t.LookupList else []):
        subs = [(st.ExtensionLookupType, st.ExtSubTable) if lk.LookupType == 9 else (lk.LookupType, st) for st in lk.SubTable]
        kind = subs[0][0] if lk.LookupType == 9 and subs else lk.LookupType
        lines.append(f"lookup {i} {kind} {lk.LookupFlag} {lk.MarkFilteringSet if lk.LookupFlag & 16 else -1}")
        for k, st in subs:
            lines.append(f"subtable {k} {st.Format}")
            lines.extend(gpos_subtable(k, st))
    return lines


def gpos_subtable(k, st):
    lines = []
    if k == 1:
        for j, g in enumerate(st.Coverage.glyphs):
            lines.append(f"single {gid(g)} " + value(st.Value if st.Format == 1 else st.Value[j]))
    elif k == 2 and st.Format == 1:
        for g, ps in zip(st.Coverage.glyphs, st.PairSet):
            for r in ps.PairValueRecord:
                lines.append(f"pair {gid(g)} {gid(r.SecondGlyph)} {value(getattr(r, 'Value1', None))} {value(getattr(r, 'Value2', None))}")
    elif k == 2:
        lines.append("coverage " + ",".join(str(gid(g)) for g in st.Coverage.glyphs))
        lines.append("class1 " + classes(st.ClassDef1))
        lines.append("class2 " + classes(st.ClassDef2))
        for ci, c1 in enumerate(st.Class1Record):
            for cj, c2 in enumerate(c1.Class2Record):
                lines.append(f"cell {ci} {cj} {value(getattr(c2, 'Value1', None))} {value(getattr(c2, 'Value2', None))}")
    elif k == 3:
        for g, r in zip(st.Coverage.glyphs, st.EntryExitRecord):
            lines.append(f"cursive {gid(g)} {anchor(r.EntryAnchor)} {anchor(r.ExitAnchor)}")
    elif k in (4, 5, 6):
        marks = (st.Mark1Coverage if k == 6 else st.MarkCoverage).glyphs
        for g, r in zip(marks, (st.Mark1Array if k == 6 else st.MarkArray).MarkRecord):
            lines.append(f"mark {gid(g)} {r.Class} {anchor(r.MarkAnchor)}")
        if k == 4:
            for g, r in zip(st.BaseCoverage.glyphs, st.BaseArray.BaseRecord):
                lines.append(f"base {gid(g)} " + " ".join(anchor(x) for x in r.BaseAnchor))
        elif k == 6:
            for g, r in zip(st.Mark2Coverage.glyphs, st.Mark2Array.Mark2Record):
                lines.append(f"mark2 {gid(g)} " + " ".join(anchor(x) for x in r.Mark2Anchor))
        else:
            for g, la in zip(st.LigatureCoverage.glyphs, st.LigatureArray.LigatureAttach):
                for c, cr in enumerate(la.ComponentRecord):
                    lines.append(f"ligature {gid(g)} {c} " + " ".join(anchor(x) for x in cr.LigatureAnchor))
    return lines


def gdef_lines():
    raw("GDEF")
    t = font["GDEF"].table
    lines = [f"version {t.Version}", "classes " + classes(t.GlyphClassDef), "markclasses " + classes(t.MarkAttachClassDef)]
    if t.AttachList:
        for g, ap in zip(t.AttachList.Coverage.glyphs, t.AttachList.AttachPoint):
            lines.append(f"attach {gid(g)} " + ",".join(map(str, ap.PointIndex)))
    if t.LigCaretList:
        for g, lg in zip(t.LigCaretList.Coverage.glyphs, t.LigCaretList.LigGlyph):
            lines.append(f"carets {gid(g)} " + ",".join(f"{c.Format}:{c.CaretValuePoint if c.Format == 2 else c.Coordinate}" for c in lg.CaretValue))
    if getattr(t, "MarkGlyphSetsDef", None):
        for c in t.MarkGlyphSetsDef.Coverage:
            lines.append("markset " + ",".join(str(gid(g)) for g in c.glyphs))
    return lines


def var_lines(tag):
    raw(tag)
    t = font[tag].table
    lines = []
    st = t.VarStore
    if st is not None:
        for i, r in enumerate(st.VarRegionList.Region):
            lines.append(f"region {i} " + " ".join(f"{f2(a.StartCoord)},{f2(a.PeakCoord)},{f2(a.EndCoord)}" for a in r.VarRegionAxis))
        for i, vd in enumerate(st.VarData):
            lines.append(f"data {i} " + ",".join(map(str, vd.VarRegionIndex)))
            for j, item in enumerate(vd.Item):
                lines.append(f"item {i} {j} " + ",".join(map(str, item)))

    def m(name, mp):
        return name + (" -" if mp is None else " " + ",".join(f"{mp.mapping[g] >> 16}:{mp.mapping[g] & 0xFFFF}" for g in order))
    if tag == "MVAR":
        lines += [f"value {r.ValueTag} {r.VarIdx >> 16} {r.VarIdx & 0xFFFF}" for r in (t.ValueRecord or [])]
    elif tag == "HVAR":
        lines += [m("advance", t.AdvWidthMap), m("lsb", t.LsbMap), m("rsb", t.RsbMap)]
    else:
        lines += [m("advance", t.AdvHeightMap), m("tsb", t.TsbMap), m("bsb", t.BsbMap), m("origin", t.VOrgMap)]
    return lines


def gvar_lines():
    spec = [raw(t) for t in ("gvar", "glyf", "loca", "head", "maxp", "fvar")]
    open(os.path.join(out, "spec"), "w").write("\n".join(spec) + "\n")
    tags = [a.axisTag for a in font["fvar"].axes]
    lines = []
    for i, name in enumerate(order):
        tvs = font["gvar"].variations.get(name) or []
        if not tvs:
            continue
        lines.append(f"glyph {i}")
        for tv in tvs:
            lines.append("tuple " + " ".join(f"{a}:{f2(tv.axes[a][0])},{f2(tv.axes[a][1])},{f2(tv.axes[a][2])}" for a in tags if a in tv.axes))
            lines.append("deltas " + " ".join("-" if c is None else f"{c[0]},{c[1]}" for c in tv.coordinates))
    return lines


def trak_lines():
    raw("trak")
    t = font["trak"]
    lines = []
    for name, data in (("horiz", t.horizData), ("vert", t.vertData)):
        if not data:
            lines.append(name + " -")
            continue
        entries = list(data.values())
        sizes = list(entries[0]._map.keys()) if entries else []
        lines.append(f"{name} sizes " + ",".join(str(q16(s)) for s in sizes))
        for track, e in data.items():
            lines.append(f"{name} track {q16(track)} {e.nameIndex} " + ",".join(str(e._map[s]) for s in sizes))
    return lines


def feat_lines():
    raw("feat")
    return [f"feature {f.FeatureType} {f.FeatureFlags} {f.FeatureNameID} " + ",".join(f"{s.SettingValue}:{s.SettingNameID}" for s in f.Settings.Setting)
            for f in font["feat"].table.FeatureNames.FeatureName]


def meta_lines():
    spec = [raw("meta") if "meta" in font else "-", raw("ltag") if "ltag" in font else "-"]
    open(os.path.join(out, "spec"), "w").write("\n".join(spec) + "\n")
    lines = []
    if "meta" in font:
        # The table's own order of data maps.
        d = font.reader["meta"]
        n = int.from_bytes(d[12:16], "big")
        for i in range(n):
            tag = d[16 + 12 * i:20 + 12 * i].decode("latin-1")
            v = font["meta"].data[tag]
            lines.append(f"meta {tag} " + (v.encode("utf-8") if isinstance(v, str) else v).hex())
    if "ltag" in font:
        lines += ["ltag " + t for t in font["ltag"].tags]
    return lines


def vertical_lines():
    spec = [raw("vhea"), raw("vmtx")] + ([raw("VORG")] if "VORG" in font else [])
    open(os.path.join(out, "spec"), "w").write("\n".join(spec) + "\n")
    lines = [f"{font['vmtx'].metrics[g][0]} {font['vmtx'].metrics[g][1]}" for g in order]
    if "VORG" in font:
        v = font["VORG"]
        lines.append(f"vorg {v.defaultVertOriginY} " + ",".join(f"{gid(g)}:{y}" for g, y in v.VOriginRecords.items()))
    return lines


def base_lines():
    raw("BASE")
    b = font["BASE"].table
    lines = []

    def coord(c):
        return "-" if c is None else str(c.Coordinate)

    def mm(m):
        parts = [coord(m.MinCoord), coord(m.MaxCoord)] + [f"{r.FeatureTableTag}:{coord(r.MinCoord)}:{coord(r.MaxCoord)}" for r in (m.FeatMinMaxRecord or [])]
        return " ".join(parts)
    for name, ax in (("horiz", b.HorizAxis), ("vert", b.VertAxis)):
        if ax is None:
            lines.append(name + " -")
            continue
        lines.append(f"{name} tags " + ",".join(ax.BaseTagList.BaselineTag if ax.BaseTagList else []))
        for r in (ax.BaseScriptList.BaseScriptRecord if ax.BaseScriptList else []):
            s = r.BaseScript
            bv = s.BaseValues
            lines.append(f"{name} script {r.BaseScriptTag} {bv.DefaultIndex if bv else -1} " + (",".join(str(c.Coordinate) for c in bv.BaseCoord) if bv else ""))
            if s.DefaultMinMax:
                lines.append(f"{name} minmax {r.BaseScriptTag} default " + mm(s.DefaultMinMax))
            for lr in (s.BaseLangSysRecord or []):
                lines.append(f"{name} minmax {r.BaseScriptTag} {lr.BaseLangSysTag} " + mm(lr.MinMax))
    return lines


kind, *rest = op.split(":")
lines = {"cff": lambda: cff_lines(int(rest[0])), "gpos": gpos_lines, "gdef": gdef_lines, "var": lambda: var_lines(rest[0]), "gvar": gvar_lines,
         "trak": trak_lines, "feat": feat_lines, "meta": meta_lines, "vertical": vertical_lines, "base": base_lines}[kind]()
sys.stdout.write("\n".join(lines) + "\n")
print(len(order), file=sys.stderr)
