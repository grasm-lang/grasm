"""Writes the site's Lottie animations.

Authored here rather than exported from After Effects, for the same reason the
counts on the site are computed rather than typed: a JSON blob nobody can read
is a thing that rots. These are a few hundred lines of shape layers with
keyframed transforms, and every one of them illustrates a claim the page makes
next to it.

    scripts/site_lottie.py          # write site/anim/*.json

Colours are the stylesheet's, as 0..1 RGB, because Lottie has no notion of a
CSS variable and a hard-coded hex in two places drifts.
"""

import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "site", "anim")

W = H = 220
FPS = 30
DUR = 90


def rgb(h):
    h = h.lstrip("#")
    return [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)] + [1]


ACCENT = rgb("1a4dff")
BRAND = rgb("0a1033")
RED = rgb("ff7a55")
FAINT = rgb("8c95bf")
DIM = rgb("b9c9ff")


def val(v):
    return {"a": 0, "k": v}


def keyed(frames):
    """An animated property: [(frame, value), ...]."""
    out = []
    for i, (t, v) in enumerate(frames):
        stop = {"t": t, "s": v if isinstance(v, list) else [v]}
        if i < len(frames) - 1:
            stop["i"] = {"x": [0.2], "y": [1]}
            stop["o"] = {"x": [0.35], "y": [0]}
        out.append(stop)
    return {"a": 1, "k": out}


def transform(pos=None, scale=None, rot=None, opacity=None):
    return {
        "o": opacity or val(100),
        "r": rot or val(0),
        "p": pos or val([W / 2, H / 2, 0]),
        "a": val([0, 0, 0]),
        "s": scale or val([100, 100, 100]),
    }


def shape_layer(name, shapes, tr, start=0, end=DUR):
    return {
        "ddd": 0, "ind": shape_layer.n, "ty": 4, "nm": name, "sr": 1,
        "ks": tr, "ao": 0,
        "shapes": shapes,
        "ip": start, "op": end, "st": 0, "bm": 0,
    }


shape_layer.n = 0


def layer(name, shapes, tr, start=0, end=DUR):
    shape_layer.n += 1
    return shape_layer(name, shapes, tr, start, end)


def ellipse(size, colour, stroke=None, pos=(0, 0)):
    items = [{"ty": "el", "s": val([size, size]), "p": val(list(pos)), "nm": "e"}]
    if stroke:
        items.append({"ty": "st", "c": val(colour), "o": val(100), "w": val(stroke),
                      "lc": 2, "lj": 2, "nm": "s"})
    else:
        items.append({"ty": "fl", "c": val(colour), "o": val(100), "r": 1, "nm": "f"})
    items.append({"ty": "tr", "p": val([0, 0]), "a": val([0, 0]), "s": val([100, 100]),
                  "r": val(0), "o": val(100)})
    return [{"ty": "gr", "it": items, "nm": "g"}]


def rect(w, h, colour, radius=3, pos=(0, 0), stroke=None):
    items = [{"ty": "rc", "s": val([w, h]), "p": val(list(pos)), "r": val(radius), "nm": "r"}]
    if stroke:
        items.append({"ty": "st", "c": val(colour), "o": val(100), "w": val(stroke),
                      "lc": 2, "lj": 2, "nm": "s"})
    else:
        items.append({"ty": "fl", "c": val(colour), "o": val(100), "r": 1, "nm": "f"})
    items.append({"ty": "tr", "p": val([0, 0]), "a": val([0, 0]), "s": val([100, 100]),
                  "r": val(0), "o": val(100)})
    return [{"ty": "gr", "it": items, "nm": "g"}]


def path(points, colour, width=2.4, closed=False, trim=None):
    verts = [list(p) for p in points]
    sh = {
        "ty": "sh", "nm": "p",
        "ks": val({"i": [[0, 0]] * len(verts), "o": [[0, 0]] * len(verts),
                   "v": verts, "c": closed}),
    }
    items = [sh]
    if trim:
        items.append({"ty": "tm", "s": val(0), "e": trim, "o": val(0), "m": 1, "nm": "t"})
    items.append({"ty": "st", "c": val(colour), "o": val(100), "w": val(width),
                  "lc": 2, "lj": 2, "nm": "s"})
    items.append({"ty": "tr", "p": val([0, 0]), "a": val([0, 0]), "s": val([100, 100]),
                  "r": val(0), "o": val(100)})
    return [{"ty": "gr", "it": items, "nm": "g"}]


def doc(layers, dur=DUR):
    return {
        "v": "5.7.4", "fr": FPS, "ip": 0, "op": dur, "w": W, "h": H,
        "nm": "grasm", "ddd": 0, "assets": [], "layers": layers,
    }


# --------------------------------------------------------------- the ceiling
#
# A package sits inside a ring. An effect reaches for the edge and is stopped:
# the ring flashes, the arrow retreats. That is the capability ceiling in one
# loop, and it is the only thing on the page that needs no caption.

def ceiling():
    shape_layer.n = 0
    ls = []
    ls.append(layer("ring", ellipse(150, BRAND, stroke=2.5),
                    transform(scale=keyed([(0, [96, 96, 100]), (30, [104, 104, 100]),
                                           (44, [96, 96, 100]), (DUR, [96, 96, 100])]))))
    ls.append(layer("flash", ellipse(150, ACCENT, stroke=3),
                    transform(opacity=keyed([(0, 0), (26, 0), (34, 90), (52, 0), (DUR, 0)]),
                              scale=val([100, 100, 100]))))
    # The effect: a short arrow that grows outward, hits, and falls back.
    ls.append(layer("effect", path([(0, 0), (58, 0)], RED, 3,
                                   trim=keyed([(0, 0), (24, 100), (40, 100), (58, 0),
                                               (DUR, 0)])),
                    transform(pos=val([W / 2, H / 2, 0]))))
    ls.append(layer("core", ellipse(34, ACCENT), transform()))
    return doc(ls)


# ------------------------------------------------------------- four engines
#
# Four tracks, one answer. Dots run left to right and converge on a single
# point, which is what difftest, crosstest and wasmtest assert.

def engines():
    shape_layer.n = 0
    ls = []
    ys = [46, 88, 130, 172]
    for i, y in enumerate(ys):
        ls.append(layer(f"track{i}", path([(24, y), (168, y)], FAINT, 1.2),
                        transform(pos=val([0, 0, 0]), opacity=val(45))))
        ls.append(layer(f"join{i}", path([(168, y), (196, 110)], BRAND, 1.2),
                        transform(pos=val([0, 0, 0]), opacity=val(60))))
        off = i * 6
        ls.append(layer(f"dot{i}", ellipse(9, ACCENT),
                        transform(pos=keyed([(0 + off, [24, y, 0]), (46 + off, [168, y, 0]),
                                             (62 + off, [196, 110, 0]),
                                             (DUR, [196, 110, 0])]),
                                  opacity=keyed([(0 + off, 0), (4 + off, 100),
                                                 (60 + off, 100), (70 + off, 0),
                                                 (DUR, 0)]))))
    ls.append(layer("answer", ellipse(18, ACCENT, stroke=2.4),
                    transform(pos=val([196, 110, 0]),
                              scale=keyed([(60, [70, 70, 100]), (70, [115, 115, 100]),
                                           (80, [100, 100, 100]), (DUR, [100, 100, 100])]))))
    return doc(ls)


# ------------------------------------------------------------------- tiers
#
# Three shells closing around a value, one after another: region, generational
# reference, unsafe. Ownership as something you pick per value.

def tiers():
    shape_layer.n = 0
    ls = []
    for i, (size, colour, at) in enumerate(
            [(70, ACCENT, 0), (116, BRAND, 14), (162, FAINT, 28)]):
        ls.append(layer(f"tier{i}", ellipse(size, colour, stroke=2.2),
                        transform(
                            opacity=keyed([(at, 0), (at + 14, 100), (DUR, 100)]),
                            scale=keyed([(at, [60, 60, 100]), (at + 16, [106, 106, 100]),
                                         (at + 26, [100, 100, 100]),
                                         (DUR, [100, 100, 100])]))))
    ls.append(layer("value", rect(20, 20, ACCENT, 4, pos=(-10, -10)),
                    transform(rot=keyed([(0, 0), (DUR, 90)]))))
    return doc(ls)


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, build in [("ceiling", ceiling), ("engines", engines), ("tiers", tiers)]:
        with open(os.path.join(OUT, f"{name}.json"), "w") as f:
            json.dump(build(), f, separators=(",", ":"))
            f.write("\n")
        print(f"wrote site/anim/{name}.json")


if __name__ == "__main__":
    main()
