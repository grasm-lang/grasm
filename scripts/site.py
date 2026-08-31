#!/usr/bin/env python3
"""Builds `site/` from one template and one page list.

Fourteen pages share a rail, a head and a footer. Written out fourteen times
they drift -- a link added to one, a heading renamed in another -- which is the
same reason every count in this repository is generated rather than typed. So
the chrome is here, once, and each page is its title and its body.

No build step for whoever reads the site: this writes plain files, and the
files are what ships. No external requests either, so it serves from anywhere
and renders under a strict content policy.

    scripts/site.py            # write site/
    scripts/site.py --check    # fail if site/ is not what this would write
"""

import argparse
import glob
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "site")

# (file, title, rail section, rail label)
NAV = [
    ("index.html", "Grasm", None, None),
    ("start.html", "Getting started", "Start", "Getting started"),
    ("install.html", "Install", "Start", "Install"),
    ("playground.html", "Playground", "Start", "Playground"),
    ("tour.html", "A tour of the language", "Learn", "Tour"),
    ("effects.html", "Effects and capabilities", "Learn", "Effects"),
    ("tiers.html", "Ownership tiers", "Learn", "Ownership"),
    ("errors.html", "Errors", "Learn", "Errors"),
    ("concurrency.html", "Concurrency", "Learn", "Concurrency"),
    ("security.html", "Capabilities and the supply chain", "Learn", "Security"),
    ("language.html", "Language reference", "Reference", "Language"),
    ("library.html", "Library reference", "Reference", "Library"),
    ("packages.html", "Packages", "Reference", "Packages"),
    ("spec.html", "Specification", "Reference", "Specification"),
    ("architecture.html", "How it is built", "Reference", "Architecture"),
    ("examples.html", "Examples", "More", "Examples"),
    ("about.html", "About", "More", "About"),
    ("status.html", "Status", "More", "Status"),
    ("roadmap.html", "Roadmap", "More", "Roadmap"),
]


def facts():
    """Every number the site states, computed rather than typed.

    Two sources. The ones that can be counted from the tree -- clauses,
    findings, programs on disk -- are counted here, exactly, every time the
    site is written. The ones that only a run can know -- how many programs
    three engines agreed on -- are read from `site/facts.json`, which
    `scripts/allgates.sh` writes *after* every gate passes. So a figure on
    the site is either a fact about the files or a fact about the last green
    run, and neither is a number somebody remembered.
    """
    f = {}
    spec = " ".join(open(p).read() for p in glob.glob(os.path.join(ROOT, "spec", "*.md")))
    f["clauses"] = len(set(re.findall(r"^\*\*\[([A-Z][A-Z0-9-]+)\]\*\*", spec, re.M)))
    findings = open(os.path.join(ROOT, "ProjectGold", "FINDINGS.md")).read()
    f["findings"] = len(re.findall(r"^## F[0-9]+ ", findings, re.M))
    f["generators"] = len(glob.glob(os.path.join(ROOT, "scripts", "fuzz_*.py")))
    f["gold"] = len(glob.glob(os.path.join(ROOT, "ProjectGold", "*", "main.gr")))
    lines = 0
    for src in glob.glob(os.path.join(ROOT, "ProjectGold", "*", "*.gr")):
        lines += open(src).read().count("\n")
    f["gold_lines"] = f"{lines:,}"
    lib = open(os.path.join(ROOT, "spec", "library.md")).read()
    f["methods"] = len(set(re.findall(r"^\| `[a-z]+\.([a-z_0-9]+)\(", lib, re.M)))

    # Stability is a promise, so the site states its size from the file that
    # makes it rather than from a sentence somebody kept up to date.
    stab = open(os.path.join(ROOT, "spec", "stability.md")).read()
    for level in ("stable", "provisional", "experimental"):
        f[level] = len(re.findall(r"^- `[A-Z][A-Z0-9-]+` \u2014 " + level + r"$", stab, re.M))
    f["spec_lines"] = sum(
        open(q).read().count("\n") for q in glob.glob(os.path.join(ROOT, "spec", "*.md"))
    )
    # Packages a consumer could depend on: the `-demo` directories exist to be
    # run by the gates, not to be imported.
    f["packages"] = len([
        d for d in glob.glob(os.path.join(ROOT, "packages", "*", "grasm.toml"))
        if not os.path.basename(os.path.dirname(d)).endswith("-demo")
    ])
    f["gates"] = len(re.findall(
        r"^run_gate ", open(os.path.join(ROOT, "scripts", "allgates.sh")).read(), re.M))
    f["version"] = re.search(
        r'^version = "([^"]+)"', open(os.path.join(ROOT, "Cargo.toml")).read(), re.M).group(1)

    measured = os.path.join(OUT, "facts.json")
    if os.path.exists(measured):
        f.update(json.load(open(measured)))
    else:
        # No green run recorded. Rather than print a number nobody measured,
        # the page says so, and `--check` fails until a run supplies it.
        for k in MEASURED:
            f.setdefault(k, "—")
    return f


# What only a run can know. `scripts/allgates.sh` writes these.
MEASURED = ["corpus", "wasm_ok", "wasm_skip", "cross", "agreeing", "laws"]


def rail(current):
    out = [
        '<a class="mark" href="index.html"><b>grasm</b>'
        "<span>a systems language</span></a>"
    ]
    section = None
    for href, _title, sec, label in NAV:
        if sec is None:
            continue
        if sec != section:
            out.append(f"<h2>{sec}</h2>")
            section = sec
        cur = ' aria-current="page"' if href == current else ""
        out.append(f'<a href="{href}"{cur}>{label}</a>')
    return "\n".join(out)


PAGE = """<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<meta name="description" content="{desc}">
<link rel="stylesheet" href="style.css">
<script type="importmap">
{{"imports": {{
  "three": "./vendor/three.module.js",
  "three/addons/utils/BufferGeometryUtils.js": "./vendor/utils/BufferGeometryUtils.js"
}}}}
</script>
<script src="vendor/motion.js" defer></script>
<script type="module" src="app.js"></script>
</head><body data-page="{page}">
<div class="frame">
<nav class="rail">
{rail}
</nav>
<main>
{body}
<footer>
Grasm — four engines, one answer. Every figure on this site is computed,
not typed.
<a href="status.html">Status</a> ·
<a href="spec.html">Specification</a> ·
<a href="roadmap.html">Roadmap</a>
</footer>
</main>
</div>
</body></html>
"""


def write(pages, check):
    os.makedirs(OUT, exist_ok=True)
    stale = []
    known = facts()
    unknown = set()

    def fill(text):
        def one(m):
            key = m.group(1)
            if key not in known:
                unknown.add(key)
                return m.group(0)
            return str(known[key])
        return re.sub(r"\u00ab([a-z_]+)\u00bb", one, text)

    for href, title, _sec, _label in NAV:
        desc, body = pages[href]
        html = PAGE.format(
            title=title, desc=desc, page=href.replace(".html", ""),
            rail=rail(href), body=fill(body)
        )
        path = os.path.join(OUT, href)
        if check:
            have = open(path).read() if os.path.exists(path) else ""
            if have != html:
                stale.append(href)
        else:
            open(path, "w").write(html)
    return stale, unknown


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    from site_pages import PAGES  # noqa: E402

    missing = [h for h, *_ in NAV if h not in PAGES]
    if missing:
        print("pages in the navigation with no content:", " ".join(missing))
        sys.exit(1)
    extra = [h for h in PAGES if h not in {n[0] for n in NAV}]
    if extra:
        print("content with no place in the navigation:", " ".join(extra))
        sys.exit(1)

    unmeasured = [k for k in MEASURED if facts().get(k) == "\u2014"]
    if unmeasured and args.check:
        print("no gate run has been recorded, so the site states no figure for:")
        print("  " + " ".join(unmeasured))
        print("run scripts/allgates.sh, which writes site/facts.json when green")
        sys.exit(1)

    stale, unknown = write(PAGES, args.check)
    if unknown:
        print("the site names a figure nothing computes:", " ".join(sorted(unknown)))
        sys.exit(1)
    if args.check and stale:
        print("site/ is not what scripts/site.py would write:")
        for s in stale:
            print(f"  {s}")
        print("run scripts/site.py")
        sys.exit(1)
    print(f"pages: {len(NAV)}   {'checked' if args.check else 'written'}")


if __name__ == "__main__":
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    main()
