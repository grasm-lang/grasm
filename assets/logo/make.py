#!/usr/bin/env python3
"""Draw the Grasm logo set into this directory.

    python3 assets/logo/make.py

The mark is a monoline "g": a closed ring and the stem that holds it, whose
descender turns into a flat bar, like a cursor. The wordmark is drawn on the
same grid, so the "g" of the wordmark is the mark. Every letter is one stroke
of width 10 with round ends; the x-height is 48 (outer edge to outer edge),
so stroke centres run from y=5 to y=43, and the descender ends at y=81.

Nothing here depends on a font: the letters are paths, so the logo looks the
same wherever it is shown. The two PNGs (an avatar and a 1280x640 social
preview, for the places that refuse SVG) are rendered from the SVGs this
writes, on macOS:

    qlmanage -t -s 512 -o . grasm-avatar.svg
    qlmanage -t -s 1280 -o . grasm-social.svg
    sips --cropToHeightWidth 640 1280 grasm-social.svg.png --out grasm-social.png
"""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
W = 10         # stroke width
HEIGHT = 81    # x-height plus descender, outer
STEM = 43.0    # the stem's centre: on the ring's right edge

DARK = ("#5BF0AE", "#17A2A2")   # on dark grounds: the site's palette
LIGHT = ("#0E9F6A", "#0B7A82")  # on light grounds: darker, for contrast
GROUND = "#0A101C"


def ring_stem(x0, descender):
    """The "g" (with a descender) or the "a" (without): a ring and a stem."""
    sx = STEM + x0
    p = f'<circle cx="{x0 + 24:g}" cy="24" r="19"/>'
    if descender:
        p += f'<path d="M{sx:g} 5 V57 A19 19 0 0 1 {sx - 19:g} 76 H{x0 + 9:g}"/>'
    else:
        p += f'<path d="M{sx:g} 5 V43"/>'
    return p, STEM + W / 2


def r(x0):
    return f'<path d="M{x0 + 5:g} 43 V24 A19 19 0 0 1 {x0 + 24:g} 5 H{x0 + 29:g}"/>', 34


def s(x0):
    return (f'<path d="M{x0 + 33:g} 5 H{x0 + 15:g} A9.5 9.5 0 0 0 {x0 + 15:g} 24 '
            f'H{x0 + 23:g} A9.5 9.5 0 0 1 {x0 + 23:g} 43 H{x0 + 5:g}"/>', 38)


def m(x0):
    return (f'<path d="M{x0 + 5:g} 43 V21 A16 16 0 0 1 {x0 + 37:g} 21 V43 '
            f'M{x0 + 37:g} 21 A16 16 0 0 1 {x0 + 69:g} 21 V43"/>', 74)


def wordmark():
    """The letters of "grasm" and the width they take."""
    parts = []
    g, w = ring_stem(0, True); parts.append(g); x = w + 12
    rr, w = r(x); parts.append(rr); x += w + 9
    a, w = ring_stem(x, False); parts.append(a); x += w + 12
    ss, w = s(x); parts.append(ss); x += w + 12
    mm, w = m(x); parts.append(mm); x += w
    return "".join(parts), x


def gradient(gid, colours, x2, y2):
    c0, c1 = colours
    return (f'<linearGradient id="{gid}" x1="0" y1="0" x2="{x2:g}" y2="{y2:g}" '
            f'gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{c0}"/>'
            f'<stop offset="1" stop-color="{c1}"/></linearGradient>')


def strokes(body, gid):
    return (f'<g fill="none" stroke="url(#{gid})" stroke-width="{W}" '
            f'stroke-linecap="round" stroke-linejoin="round">{body}</g>')


def write(name, svg):
    with open(os.path.join(HERE, name), "w") as f:
        f.write(svg + "\n")


def main():
    body, width = wordmark()
    pad = 6
    vw, vh = width + 2 * pad, HEIGHT + 2 * pad
    for ground, colours in (("dark", DARK), ("light", LIGHT)):
        write(f"grasm-wordmark-on-{ground}.svg",
              f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{-pad} {-pad} {vw:g} {vh:g}" '
              f'width="{vw * 2:g}" height="{vh * 2:g}" role="img" aria-label="Grasm">'
              f'<defs>{gradient("g", colours, width, HEIGHT)}</defs>{strokes(body, "g")}</svg>')

    mark, mw = ring_stem(0, True)
    k = 2.0
    place = f'translate({128 - k * mw / 2:g} {128 - k * HEIGHT / 2:g}) scale({k:g})'
    inner = (f'<defs>{gradient("g", DARK, mw, HEIGHT)}</defs>'
             f'<g transform="{place}">{strokes(mark, "g")}</g>')
    # The mark on its own tile, for a favicon or an app icon.
    write("grasm-mark.svg",
          f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256" '
          f'role="img" aria-label="Grasm"><rect width="256" height="256" rx="56" fill="{GROUND}"/>'
          f'<rect x="1" y="1" width="254" height="254" rx="55" fill="none" stroke="#1C2838" '
          f'stroke-width="2"/>{inner}</svg>')
    # Full bleed, for an avatar: the site that shows it rounds the corners.
    write("grasm-avatar.svg",
          f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="512" height="512" '
          f'role="img" aria-label="Grasm"><rect width="256" height="256" fill="{GROUND}"/>{inner}</svg>')

    # The social preview: 1280x640, drawn in the middle of a square because
    # qlmanage renders squares; the PNG is cropped back to 1280x640.
    ww = 620
    wh = ww * vh / vw
    write("grasm-social.svg",
          f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -320 1280 1280" width="1280" '
          f'height="1280"><rect y="-320" width="1280" height="1280" fill="{GROUND}"/>'
          f'<svg x="140" y="150" width="{ww}" height="{wh:.1f}" viewBox="{-pad} {-pad} {vw:g} {vh:g}">'
          f'<defs>{gradient("g", DARK, width, HEIGHT)}</defs>{strokes(body, "g")}</svg>'
          f'<text x="148" y="452" font-family="-apple-system,Helvetica Neue,Helvetica,sans-serif" '
          f'font-size="44" font-weight="600" fill="#EAF2FA">Nothing happens that the code '
          f'didn&#8217;t declare.</text>'
          f'<text x="150" y="508" font-family="Menlo,monospace" font-size="24" fill="#6E829A">'
          f'effects you can audit &#183; ownership without lifetimes &#183; no collector</text></svg>')


if __name__ == "__main__":
    main()
