#!/usr/bin/env python3
"""Build `shapes.ttf`, the font `packages/pcfont` is tested against.

Generated rather than vendored: a system font is licensed, and a font whose
outlines are written here is one whose expected coverage can be stated
exactly. Every glyph is axis-aligned or a straight diagonal at a round
em-size, so the right answer for a rasteriser is arithmetic rather than a
picture somebody eyeballed.

    python3 scripts/fixtures/font/make.py

Needs fontTools. The output is committed, so this only runs when the shapes
change.
"""

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
import os

UPM = 1000

def square(pen):
    # A 500x500 box from (100,0) to (600,500): a solid interior, so coverage
    # in the middle must be full.
    pen.moveTo((100, 0))
    pen.lineTo((600, 0))
    pen.lineTo((600, 500))
    pen.lineTo((100, 500))
    pen.closePath()

def ring(pen):
    # A box with a box-shaped hole, wound the other way: the hole must be
    # empty, which is what tells a nonzero fill from a broken one.
    pen.moveTo((0, 0))
    pen.lineTo((800, 0))
    pen.lineTo((800, 800))
    pen.lineTo((0, 800))
    pen.closePath()
    pen.moveTo((200, 200))
    pen.lineTo((200, 600))
    pen.lineTo((600, 600))
    pen.lineTo((600, 200))
    pen.closePath()

def triangle(pen):
    # A right triangle: the diagonal is where anti-aliasing shows.
    pen.moveTo((0, 0))
    pen.lineTo((800, 0))
    pen.lineTo((0, 800))
    pen.closePath()

def curve(pen):
    # A quadratic arc, so the Bezier flattening is exercised.
    pen.moveTo((0, 0))
    pen.qCurveTo((400, 800), (800, 0))
    pen.closePath()

def nested(pen):
    # Two boxes wound the SAME way. This is the only shape here that tells
    # nonzero winding from even-odd: with alternating directions the running
    # sum is only ever 0 or 1, where the two rules agree. Here the inner box
    # takes it to 2, so nonzero fills solid and even-odd punches a hole.
    pen.moveTo((0, 0))
    pen.lineTo((800, 0))
    pen.lineTo((800, 800))
    pen.lineTo((0, 800))
    pen.closePath()
    pen.moveTo((200, 200))
    pen.lineTo((600, 200))
    pen.lineTo((600, 600))
    pen.lineTo((200, 600))
    pen.closePath()

def narrow(pen):
    pen.moveTo((300, 0))
    pen.lineTo((400, 0))
    pen.lineTo((400, 700))
    pen.lineTo((300, 700))
    pen.closePath()

SHAPES = [
    (".notdef", None, None, 600),
    ("space", " ", None, 600),
    ("square", "A", square, 700),
    ("ring", "B", ring, 900),
    ("triangle", "C", triangle, 900),
    ("curve", "D", curve, 900),
    ("narrow", "E", narrow, 500),
    ("nested", "F", nested, 900),
]

def main():
    fb = FontBuilder(UPM, isTTF=True)
    names = [s[0] for s in SHAPES]
    fb.setupGlyphOrder(names)
    cmap = {}
    glyphs = {}
    metrics = {}
    for name, ch, draw, adv in SHAPES:
        pen = TTGlyphPen(None)
        if draw is not None:
            draw(pen)
        glyphs[name] = pen.glyph()
        metrics[name] = (adv, 0)
        if ch is not None:
            cmap[ord(ch)] = name
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({
        "familyName": "Grasm Shapes",
        "styleName": "Regular",
        "psName": "GrasmShapes-Regular",
    })
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200)
    fb.setupPost()
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "shapes.ttf")
    fb.save(out)
    print(f"wrote {out}")

if __name__ == "__main__":
    main()
