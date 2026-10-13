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
    ("different.html", "What makes it different", "Start", "Why Grasm"),
    ("start.html", "Getting started", "Start", "Getting started"),
    ("install.html", "Install", "Start", "Install"),
    ("playground.html", "Playground", "Start", "Playground"),
    ("tutorial.html", "Grasm in an hour", "Learn", "Tutorial"),
    ("tour.html", "A tour of the language", "Learn", "Tour"),
    ("effects.html", "Effects and capabilities", "Learn", "Effects"),
    ("tiers.html", "Ownership tiers", "Learn", "Ownership"),
    ("errors.html", "Errors", "Learn", "Errors"),
    ("concurrency.html", "Concurrency", "Learn", "Concurrency"),
    ("security.html", "Capabilities and the supply chain", "Learn", "Security"),
    ("syntax.html", "Syntax", "Reference", "Syntax"),
    ("language.html", "Language reference", "Reference", "Language"),
    ("library.html", "Library reference", "Reference", "Library"),
    ("packages.html", "Packages", "Reference", "Packages"),
    ("spec.html", "Specification", "Reference", "Specification"),
    ("architecture.html", "How it is built", "Reference", "Architecture"),
    ("examples.html", "Examples", "More", "Examples"),
    ("achieved.html", "How far it got", "More", "Scorecard"),
    ("performance.html", "Where the time goes", "More", "Performance"),
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
    f["spec_lines"] = "{:,}".format(sum(
        open(q).read().count("\n") for q in glob.glob(os.path.join(ROOT, "spec", "*.md"))
    ))
    # Packages a consumer could depend on: the `-demo` directories exist to be
    # run by the gates, not to be imported.
    f["packages"] = len([
        d for d in glob.glob(os.path.join(ROOT, "packages", "*", "grasm.toml"))
        if not os.path.basename(os.path.dirname(d)).endswith("-demo")
    ])
    # How many of those declare nothing at all. The site's whole argument is
    # that reach is separable from computation, and this is the one number that
    # says whether the library set actually does that or only could.
    f["packages_pure"] = len([
        d for d in glob.glob(os.path.join(ROOT, "packages", "*", "grasm.toml"))
        if not os.path.basename(os.path.dirname(d)).endswith("-demo")
        and (lambda m: bool(m) and not m.group(1).strip())(
            re.search(r"^capabilities\s*=\s*\[(.*?)\]", open(d).read(), re.M | re.S))
    ])
    f["packages_reaching"] = f["packages"] - f["packages_pure"]
    # And which capability the rest ask for, because "96 reach something" and
    # "one of 651 may call into C" are different facts about the same set.
    for cap in ("io", "net", "unsafe"):
        f["cap_" + cap] = len([
            d for d in glob.glob(os.path.join(ROOT, "packages", "*", "grasm.toml"))
            if not os.path.basename(os.path.dirname(d)).endswith("-demo")
            and (lambda m: bool(m) and ('"%s"' % cap) in m.group(1))(
                re.search(r"^capabilities\s*=\s*\[(.*?)\]",
                          open(d).read(), re.M | re.S))
        ])
    f["gates"] = len(re.findall(
        r"^run_gate ", open(os.path.join(ROOT, "scripts", "allgates.sh")).read(), re.M))
    f["version"] = re.search(
        r'^version = "([^"]+)"', open(os.path.join(ROOT, "Cargo.toml")).read(), re.M).group(1)

    f["conformance"] = len(glob.glob(os.path.join(ROOT, "spec", "conformance", "*.gr")))
    # The examples page names three programs by their size, and one of them had
    # grown thirty lines since somebody typed it.
    for key, where in (("lines_http", "21-http-server"),
                       ("lines_sql", "28-sql-engine"),
                       ("lines_json", "13-json-parser")):
        with open(os.path.join(ROOT, "ProjectGold", where, "main.gr")) as fh:
            f[key] = fh.read().count("\n")
    f["apps"] = len(glob.glob(os.path.join(ROOT, "apps", "*", "grasm.toml")))
    f["examples"] = len(glob.glob(os.path.join(ROOT, "examples", "*.gr")))
    f.update(cited())
    f.update(derived(f))
    f.update(scorecard())

    # Icons travel the same substitution path as the figures, so a page can
    # name one and the build fails if it does not exist.
    for name in ICON:
        f["icon_" + name] = icon(name)

    measured = os.path.join(OUT, "facts.json")
    if os.path.exists(measured):
        f.update(json.load(open(measured)))
    else:
        # No green run recorded. Rather than print a number nobody measured,
        # the page says so, and `--check` fails until a run supplies it.
        for k in MEASURED:
            f.setdefault(k, "—")
    return f


# A figure some document in this repository recorded, read out of that document
# rather than retyped here. A measurement belongs where it was taken -- the
# north-star scorecard and the size record are written by whoever took the
# reading -- and the site reads it from there. If a record changes shape the
# pattern stops matching and the build fails naming it, which is the failure
# mode a retyped number does not have: that one just goes quietly stale.
CITED = [
    ("geomean", "docs/northstar.md", r"\| G2 \|[^|]*\|[^|]*\| \*\*([0-9.]+)×\*\*"),
    ("geomean_floor", "docs/northstar.md", r"and \*\*([0-9.]+)×\*\* with the process floor"),
    ("locs_low", "docs/northstar.md", r"\*\*([0-9]+)–[0-9]+k\*\* across consecutive runs"),
    ("locs_high", "docs/northstar.md", r"\*\*[0-9]+–([0-9]+)k\*\* across consecutive runs"),
    ("incr_p50", "docs/northstar.md", r"\*\*p50 ([0-9]+) ms, p95 [0-9]+ ms\*\*"),
    ("incr_p95", "docs/northstar.md", r"\*\*p50 [0-9]+ ms, p95 ([0-9]+) ms\*\*"),
    ("hello_kb", "docs/northstar.md", r"\*\*met, ([0-9.]+) KB on [0-9-]+\*\*"),
    ("hello_date", "docs/northstar.md", r"\*\*met, [0-9.]+ KB on ([0-9-]+)\*\*"),
    ("hello_was", "docs/northstar.md", r"It was ([0-9.]+) KB, of which"),
    ("tasks", "docs/northstar.md", r"\*\*([0-9,]+) tasks run in [0-9]+ s\*\*"),
    ("tasks_s", "docs/northstar.md", r"\*\*[0-9,]+ tasks run in ([0-9]+) s\*\*"),
    ("tasks_was", "docs/northstar.md", r"([0-9,]+) tasks took [0-9]+ s"),
    ("tasks_was_s", "docs/northstar.md", r"[0-9,]+ tasks took ([0-9]+) s"),
    ("interp_tasks", "docs/northstar.md",
     r"interpreter stops near ([0-9]{1,3}(?:,[0-9]{3})*)"),
    ("tokens_py", "docs/northstar.md", r"\| Tokens vs Python \|[^|]*\| \*\*([0-9.]+)×\*\*"),
    ("tokens_go", "docs/northstar.md", r"\| Tokens vs Go \|[^|]*\| \*\*([0-9.]+)×\*\*"),
    ("replay_builtins", "docs/northstar.md", r"All \*\*([0-9]+)\*\* journaled builtins"),
    ("leak_programs", "docs/binary-size.md", r"\| .leakcheck. \| ([0-9]+) programs"),
    ("use_programs", "docs/binary-size.md", r"\| .usecheck.[^|]*\| ([0-9]+) programs"),
    ("cross_total", "docs/binary-size.md", r"\| .crosstest. x86_64 \| ([0-9]+) programs"),
    ("cross_pass", "docs/binary-size.md", r"\*\*([0-9]+) pass, 0 real failures"),
    ("unittests", "docs/binary-size.md", r"\| unit tests \| every crate \| ([0-9]+) pass"),
    ("difftest", "docs/binary-size.md", r"\| difftest / corpus comparison \| ([0-9]+) programs"),
    ("linux_lines", "docs/binary-size.md", r"roughly ([0-9,]+) lines of .cfg\(not\(macos\)\)."),
    ("tree_ratio", "docs/northstar.md", r"`tree` at ([0-9.]+)\u00d7 is what the geomean"),
    ("geomean_was", "docs/northstar.md", r"geometric mean is ([0-9.]+)\u00d7 behind"),
    ("symbolizer_kb", "docs/northstar.md", r"about ([0-9]+) KB was Rust's own backtrace"),
    ("etl_grasm_ms", "benchmarks/systems/RESULTS.md",
     r"\| etl \(1 GiB\) \| wall ms \| ([0-9,]+)"),
    ("etl_rust_ms", "benchmarks/systems/RESULTS.md",
     r"\| etl \(1 GiB\) \| wall ms \| [0-9,]+ \| \*\*([0-9,]+)\*\*"),
    ("chat_rss", "benchmarks/systems/RESULTS.md",
     r"\| chat \| peak RSS MB \| \*\*([0-9.]+)\*\*"),
    ("chat_rss_rust", "benchmarks/systems/RESULTS.md",
     r"\| chat \| peak RSS MB \| \*\*[0-9.]+\*\* \| ([0-9.]+)"),
]


def derived(f):
    """Ratios the site states, divided here rather than on the page.

    A ratio typed into prose outlives the two numbers it came from -- that is
    what happened to the 21 ms benchmark artefact, which was published in three
    places and corrected in one. These are computed from the cited figures, so
    re-measuring either side moves the sentence.
    """
    def num(key):
        return float(f[key].replace(",", ""))
    return {
        "etl_ratio": "{:.1f}".format(num("etl_grasm_ms") / num("etl_rust_ms")),
        "chat_mem_ratio": "{:.0f}".format(num("chat_rss_rust") / num("chat_rss")),
    }


def cited():
    """Each CITED figure, read from the document that recorded it."""
    out = {}
    for key, doc, pattern in CITED:
        text = open(os.path.join(ROOT, doc)).read()
        hit = re.search(pattern, text)
        if not hit:
            raise SystemExit(
                f"{doc} no longer states the figure the site calls "
                f"«{key}»: nothing matches {pattern!r}.\n"
                "Either the record moved, and the pattern in scripts/site.py "
                "wants fixing, or the measurement was dropped -- in which case "
                "the page that states it has to go too."
            )
        out[key] = hit.group(1)
    return out


def scorecard():
    """The scorecard's shape, counted from its own table.

    A row is met, narrowly missed, or waiting on something no gate can supply.
    "Nine of thirteen" is then a count of that table rather than a claim about
    it, and a row that moves moves the summary with it.
    """
    rows = re.findall(
        r"^\| (G[0-9]+) \| ([^|]+?) \| ([^|]+?) \| .* \| (.+?) \|$",
        open(os.path.join(ROOT, "docs", "northstar.md")).read(), re.M)
    if len(rows) < 10:
        raise SystemExit("docs/northstar.md no longer has a scorecard table the "
                         f"site can read: found {len(rows)} rows, expected 13")
    met = over = blocked = 0
    for _id, _goal, _target, verdict in rows:
        plain = verdict.replace("*", "").strip().lower()
        if plain.startswith("met") or plain.startswith("strongest"):
            met += 1
        elif "over" in plain:
            over += 1
        else:
            blocked += 1
    return {"goals_total": len(rows), "goals_met": met,
            "goals_over": over, "goals_blocked": blocked}


# What only a run can know. `scripts/allgates.sh` writes these.
MEASURED = ["corpus", "wasm_ok", "wasm_skip", "cross", "agreeing", "laws"]



# Line icons, drawn here rather than pulled from a set, so the site keeps its
# "no external request" property and every glyph is 16 lines of SVG anybody can
# read. 1.6px stroke on a 24 grid, `currentColor` so CSS decides the colour.
ICON = {
    "mark": '<path d="M12 3 4 7.5v9L12 21l8-4.5v-9L12 3Z"/><path d="M12 12 4 7.5M12 12l8-4.5M12 12v9"/>',
    "shield": '<path d="M12 3 5 6v6c0 4 3 6.5 7 9 4-2.5 7-5 7-9V6l-7-3Z"/><path d="m9 12 2 2 4-4"/>',
    "layers": '<path d="m12 3 9 5-9 5-9-5 9-5Z"/><path d="m3 13 9 5 9-5"/>',
    "cpu": '<rect x="8" y="8" width="8" height="8" rx="1"/><rect x="4" y="4" width="16" height="16" rx="2"/><path d="M9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2"/>',
    "check": '<path d="m4 12 5 5L20 6"/>',
    "x": '<path d="M6 6l12 12M18 6 6 18"/>',
    "book": '<path d="M4 5a2 2 0 0 1 2-2h13v16H6a2 2 0 0 0-2 2V5Z"/><path d="M8 7h7M8 11h7"/>',
    "terminal": '<path d="m6 8 4 4-4 4M13 16h5"/><rect x="2" y="4" width="20" height="16" rx="2"/>',
    "flow": '<path d="M4 6h6M4 12h9M4 18h6"/><circle cx="18" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><circle cx="19" cy="12" r="2"/>',
    "lock": '<rect x="4" y="10" width="16" height="10" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>',
    "gauge": '<path d="M12 14 16 9"/><path d="M4 18a9 9 0 1 1 16 0"/>',
    "package": '<path d="m12 3 8 4.5v9L12 21l-8-4.5v-9L12 3Z"/><path d="M4 7.5 12 12l8-4.5M12 12v9"/>',
}


def icon(name, size=17):
    """One icon, inline. Unknown names fail the build rather than render blank."""
    if name not in ICON:
        raise SystemExit(f"no icon called {name!r}")
    return (
        f'<svg viewBox="0 0 24 24" width="{size}" height="{size}" fill="none" '
        f'stroke="currentColor" stroke-width="1.6" stroke-linecap="round" '
        f'stroke-linejoin="round" aria-hidden="true">{ICON[name]}</svg>'
    )


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
<script src="vendor/motion.js" defer></script>
<script src="vendor/lottie.js" defer></script>
<script type="module" src="app.js"></script>
</head><body data-page="{page}">
<header class="topbar">
  <a class="brand" href="index.html">{markicon}<span>grasm</span></a>
  <span class="ver">v{version}</span>
  <nav>
    <a href="different.html">Why</a>
    <a href="start.html">Docs</a>
    <a href="syntax.html">Syntax</a>
    <a href="security.html">Security</a>
    <a href="achieved.html">Scorecard</a>
    <a href="status.html">Status</a>
  </nav>
</header>
<div class="frame">
<nav class="rail">
{rail}
</nav>
<main>
{body}
<footer>
Grasm — four engines, one answer. Counts here are computed from the
repository as the page is written; measurements are read from the document
that recorded them.
<a href="achieved.html">Scorecard</a> ·
<a href="performance.html">Performance</a> ·
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
        # Anything between the marks, not just a well-formed name. Twice now a
        # placeholder has gone out unsubstituted because it fell outside a
        # narrower class -- `incr_p95` for its digit, then a stray `\u00d7`
        # inside the marks -- and in both cases the guard below said nothing,
        # because it only ever saw the names this pattern had matched.
        return re.sub(r"\u00ab([^\u00bb\n]{1,48})\u00bb", one, text)

    for href, title, _sec, _label in NAV:
        desc, body = pages[href]
        # The description is substituted too. It was not, and a page's
        # own `<meta>` sat two dozen clauses out of date where the body it
        # described was right.
        html = PAGE.format(
            title=title, desc=fill(desc), page=href.replace(".html", ""),
            markicon=icon("mark", 19), version=known.get("version", ""),
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


# The playground's examples, in the order its menu lists them. Read from
# `examples/` rather than written into the page, so a program shown there is
# the file the gates run -- not a copy that can stop compiling unnoticed.
PLAYGROUND_EXAMPLES = [
    ("hello", "Hello"),
    ("errors", "Errors as values"),
    ("effects", "Effects and handlers"),
    ("generics", "Generics"),
    ("function_values", "Function values"),
    ("combinators", "Combinators"),
    ("ranges", "Ranges"),
    ("ownership", "Ownership"),
]

# The browser engine. `scripts/wasmtest.sh` holds this exact module to the
# other engines, so the site serves that file and never a build of its own.
WASM_SRC = os.path.join(ROOT, "ProjectGold", "web", "grasm.wasm")


def playground_assets():
    """What `playground.js` loads, as (name, bytes)."""
    examples = []
    for name, title in PLAYGROUND_EXAMPLES:
        with open(os.path.join(ROOT, "examples", name + ".gr")) as f:
            examples.append({"id": name, "title": title, "src": f.read()})
    text = json.dumps(examples, indent=1, ensure_ascii=False) + "\n"
    with open(WASM_SRC, "rb") as f:
        wasm = f.read()
    return [("playground.json", text.encode()), ("grasm.wasm", wasm)]


def write_assets(check):
    stale = []
    for name, data in playground_assets():
        path = os.path.join(OUT, name)
        if check:
            have = open(path, "rb").read() if os.path.exists(path) else b""
            if have != data:
                stale.append(name)
        else:
            with open(path, "wb") as f:
                f.write(data)
    return stale


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

    # The animations are generated too, for the same reason the counts are:
    # a hand-edited JSON blob nobody can read is a thing that rots.
    if not args.check:
        import site_lottie
        site_lottie.main()

    stale, unknown = write(PAGES, args.check)
    stale += write_assets(args.check)
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
