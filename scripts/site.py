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
import ast
import glob
import datetime
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
    # Libraries a consumer could depend on: a package that exports a `pub fn`.
    # The `-demo` directories exist to be run by the gates, and so do the
    # test harnesses (`b47test`, `uritdiff`, ...) -- programs with a `main` and
    # no API -- so neither is counted. Counting every directory said 1000 when
    # 891 were libraries.
    libs = library_manifests()
    f["packages"] = len(libs)
    # How many of those declare nothing at all. The site's whole argument is
    # that reach is separable from computation, and this is the one number that
    # says whether the library set actually does that or only could.
    caps = {d: (lambda m: m.group(1) if m else None)(
        re.search(r"^capabilities\s*=\s*\[(.*?)\]", open(d).read(), re.M | re.S)) for d in libs}
    f["packages_pure"] = len([d for d in libs if caps[d] is not None and not caps[d].strip()])
    f["packages_reaching"] = f["packages"] - f["packages_pure"]
    # And which capability the rest ask for, because "96 reach something" and
    # "one of 651 may call into C" are different facts about the same set.
    for cap in ("io", "net", "unsafe"):
        f["cap_" + cap] = len([d for d in libs if caps[d] is not None and ('"%s"' % cap) in caps[d]])
    f["gates"] = gate_count()
    f["version"] = re.search(
        r'^version = "([^"]+)"', open(os.path.join(ROOT, "Cargo.toml")).read(), re.M).group(1)

    f["conformance"] = len(glob.glob(os.path.join(ROOT, "spec", "conformance", "*.gr")))
    f["programs"] = corpus_programs()
    f["law_defs"] = law_count()
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
        record = json.load(open(measured))
        # `taken` is a date per figure, not a figure. A gate is recorded the
        # moment it passes, so two figures in one file can be months apart.
        taken = record.pop("taken", {})
        f.update(record)
    else:
        # No green run recorded. Rather than print a number nobody measured,
        # the page says so, and `--check` fails until a run supplies it.
        taken = {}
        for k in MEASURED:
            f.setdefault(k, "\u2014")
    f.update(vintage(f, taken))
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
    # The spec page showed "70 -- pages, the cap". 70 is not the cap: the cap
    # is 50 and the specification is 40% over it, which is the opposite claim.
    ("spec_pages", "spec/core.md",
     r"\| Specification \| \u2264 [0-9]+ pages \| \*\*~([0-9]+) pages\*\* \|"),
    ("spec_cap", "spec/core.md",
     r"\| Specification \| \u2264 ([0-9]+) pages \|"),
    ("spec_constructs", "spec/core.md",
     r"\| Core constructs \| \u2264 [0-9]+ \| \*\*([0-9]+)\*\* \|"),
    ("spec_constructs_cap", "spec/core.md",
     r"\| Core constructs \| \u2264 ([0-9]+) \|"),
    ("linux_lines", "docs/binary-size.md", r"roughly ([0-9,]+) lines of .cfg\(not\(macos\)\)."),
    ("tree_ratio", "docs/northstar.md", r"`tree` at ([0-9.]+)\u00d7 is what the geomean"),
    # Past tense on purpose: the sentence is dated 2026-08-19 and sits under
    # a row recording 0.965x, so a pattern demanding "is" would pin the
    # document to the wrong tense just to keep the citation alive.
    ("geomean_was", "docs/northstar.md", r"geometric mean was ([0-9.]+)\u00d7 behind"),
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
        # How far past its own page budget the specification is. Typed as
        # "40%" first, which is a quotient of two cited figures and so the
        # exact thing that outlives them.
        "spec_over": "{:.0f}".format(
            (num("spec_pages") / num("spec_cap") - 1) * 100),
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
            "goals_over": over, "goals_blocked": blocked,
            # Eleven goals in thirteen rows, because G3 and G11 each split in
            # half. Both numbers were typed on the index; a twelfth goal would
            # have moved one and not the other.
            "goals_distinct": len({r[0] for r in rows}),
            "goals_split": len(rows) - len({r[0] for r in rows})}


def library_manifests():
    """The grasm.toml of every package that is a library: not a `-demo`, and
    exporting at least one `pub fn` from src/main.gr."""
    out = []
    for d in sorted(glob.glob(os.path.join(ROOT, "packages", "*", "grasm.toml"))):
        pkg = os.path.dirname(d)
        main = os.path.join(pkg, "src", "main.gr")
        if pkg.endswith("-demo") or not os.path.exists(main):
            continue
        if re.search(r"^pub fn ", open(main).read(), re.M):
            out.append(d)
    return out


def gate_count():
    """Every gate a green `allgates.sh` run executes.

    Counting lines that start with `run_gate` understated this by seventeen.
    Three `run_gate` calls are indented, and an indented one is not a smaller
    gate: fifteen fuzzers are *one* line expanded from a glob, and `sitefacts`
    and `site` sit behind the "nothing failed" branch, which is exactly the
    branch a figure on the site was written during.
    """
    src = open(os.path.join(ROOT, "scripts", "allgates.sh")).read()
    calls = len(re.findall(r"^\s*run_gate ", src, re.M))
    loop = re.search(r"^for f in (scripts/fuzz_\*\.py); do\n\s*run_gate ", src, re.M)
    if not loop:
        raise SystemExit(
            "scripts/allgates.sh no longer expands the fuzzers from a glob the "
            "way the site counts them. Every page that states a gate count "
            "reads this; fix the pattern in scripts/site.py rather than "
            "letting the count quietly drop fifteen gates.")
    fuzzers = glob.glob(os.path.join(ROOT, loop.group(1)))
    if not fuzzers:
        raise SystemExit(
            "scripts/allgates.sh globs %s and nothing matches, so the gate "
            "count would silently lose the fuzzers." % loop.group(1))
    # The loop's own `run_gate` line is one call and len(fuzzers) gates.
    return calls - 1 + len(fuzzers)


def corpus_programs():
    """How many standalone programs the corpus holds, counted now.

    This was read out of `docs/binary-size.md`, which is where difftest's last
    run recorded it, and that made the size of the corpus a measurement -- so
    a program added today did not appear in the denominator until somebody ran
    a two-hour gate suite. It is a count: the filter below is difftest's own,
    and the directory list is read from the file difftest sources rather than
    repeated here, because a copied corpus definition is the exact mistake
    `scripts/lib/corpus.sh` exists to prevent.

    What still needs a run is the *agreement*, and that is `corpus` below.
    """
    lib = open(os.path.join(ROOT, "scripts", "lib", "corpus.sh")).read()
    dirs = re.search(r'CORPUS_DIRS=\$\{CORPUS_DIRS:-"([^"]+)"\}', lib)
    if not dirs:
        raise SystemExit(
            "scripts/lib/corpus.sh no longer sets CORPUS_DIRS the way the site "
            "reads it. The corpus size on every page comes from that list; "
            "fix the pattern in scripts/site.py rather than guessing the list."
        )
    n = 0
    for d in dirs.group(1).split():
        for root, _, names in os.walk(os.path.join(ROOT, d)):
            for name in names:
                if not name.endswith(".gr"):
                    continue
                text = open(os.path.join(root, name)).read()
                # difftest skips library modules and the conformance programs
                # that are meant not to compile.
                if not re.search(r"^\s*(pub )?fn main\(", text, re.M):
                    continue
                if re.search(r"^//! expect: error$", text, re.M):
                    continue
                n += 1
    return n


def law_count():
    """How many laws `scripts/fuzz_laws.py` defines, counted here.

    This had the same shape of mistake as the corpus size: the site read the
    figure out of the gate's log, so the page said 197 while the file defined
    205 -- eight laws added since the last recorded run, none of them on the
    page that exists to say how many there are. A green run of that gate
    checks every law it defines (`sys.exit(1 if broken or rejected else 0)`),
    so the count is the definition and only the *holding* needs a run.

    Parsed rather than imported, because a site generator should not execute
    a fuzzer to count a dict.
    """
    path = os.path.join(ROOT, "scripts", "fuzz_laws.py")
    tree = ast.parse(open(path).read())
    for node in tree.body:
        if (isinstance(node, ast.Assign)
                and any(getattr(t, "id", None) == "LAWS" for t in node.targets)
                and isinstance(node.value, ast.Dict)):
            return len(node.value.keys)
    raise SystemExit(
        "scripts/fuzz_laws.py no longer assigns a literal LAWS dict at the "
        "top level, so the site cannot count the laws it states. Fix the "
        "reader in scripts/site.py rather than retyping the number: a typed "
        "one was eight short for a month."
    )


def vintage(f, taken):
    """What each gate-written figure is, and when it was taken.

    A figure in `site/facts.json` is as current as the run that wrote it and
    no more, and the file carried no date -- so the one written in September
    was still being read in October under a heading saying every number came
    from the run that proved it.

    This produced a prose span over the set as well ("runs between X and Y,
    when the corpus held N"). A span is right for the six together and wrong
    for any one of them, and the corpus clause was right only while every
    figure shared a run -- which stopped being true the moment the gates began
    recording one at a time. A row per figure needs no such sentence, so the
    span is gone and nothing names it.
    """
    out = {}
    for k in MEASURED:
        out["when_" + k] = (pretty(taken[k]) if k in taken
                            else "a run that recorded no date")

    # Whether the wasm run covered the corpus as it stands, as a clause rather
    # than two numbers the reader has to compare. It read "out of the 478 the
    # corpus held then -- it holds 511 today" when that was true, and would
    # have gone on reading that way with both numbers equal.
    try:
        then = int(f["wasm_ok"]) + int(f["wasm_skip"])
        covered = then == int(f["programs"])
    except (KeyError, TypeError, ValueError):
        then, covered = None, False
    out["wasm_of"] = (
        "which is every program in the corpus" if covered else
        "out of the %s the corpus held then, against %s today"
        % ("{:,}".format(then) if then else "\u2014",
           "{:,}".format(f["programs"])))

    out["facts_rows"] = "\n".join(
        "<tr><td><code>%s</code></td><td>%s</td><td>%s</td><td>%s</td></tr>"
        % (gate, label, "{:,}".format(f[k]) if isinstance(f.get(k), int)
           else f.get(k, "\u2014"),
           pretty(taken[k]) if k in taken else "not recorded")
        for k, gate, label in FIGURE_ROWS)
    return out


def pretty(iso):
    """2026-09-13 -> 13 September 2026."""
    return datetime.date.fromisoformat(iso).strftime("%-d %B %Y")


# What only a run can know. `scripts/allgates.sh` writes these.
MEASURED = ["corpus", "wasm_ok", "wasm_skip", "cross", "agreeing", "laws",
            "depth_cap"]

# The same seven, with the gate that writes each and what it means, for the
# table on the status page.
FIGURE_ROWS = [
    ("corpus", "difftest", "interpreter, bundle and native identical"),
    ("wasm_ok", "wasmtest", "identical in the browser engine"),
    ("wasm_skip", "wasmtest", "skipped: no filesystem, socket, process or thread"),
    ("cross", "crosstest", "cross-compiled to x86_64, identical under Rosetta"),
    ("agreeing", "ProjectGold", "whole programs agreeing on every engine"),
    ("laws", "fuzz_laws", "laws that held against arithmetic"),
    ("depth_cap", "depth", "nested calls the interpreter reaches, against 10,000 promised"),
]



# Line icons, drawn here rather than pulled from a set, so the site keeps its
# "no external request" property and every glyph is 16 lines of SVG anybody can
# read. 1.6px stroke on a 24 grid, `currentColor` so CSS decides the colour.
ICON = {
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


# The mark: a hard bar, and the letter sitting under it. The bar is the
# ceiling a package declares and the compiler enforces, which is the one idea
# everything else here is built on -- so the logo is that idea and not a cube.
MARK = '<path d="M9 6H39"/><path d="M24 14A13 13 0 1 0 37 27H26"/>'


def mark(size=20, stroke=6):
    """The logo, inline. Its own viewBox: the UI icons are drawn on 24 and
    hairline, the mark on 48 and heavy, and sharing one helper made it thin."""
    return (
        f'<svg viewBox="0 0 48 48" width="{size}" height="{size}" fill="none" '
        f'stroke="currentColor" stroke-width="{stroke}" stroke-linecap="round" '
        f'stroke-linejoin="round" aria-hidden="true">{MARK}</svg>'
    )


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
        '<a class="mark" href="index.html">' + mark(18, 6) + '<b>grasm</b>'
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


def foot():
    """The footer's link columns. Same source as the rail, so a page added to
    one is in the other and neither can quietly fall behind."""
    cols, section = [], None
    for href, _title, sec, label in NAV:
        if sec is None:
            continue
        if sec != section:
            cols.append((sec, []))
            section = sec
        cols[-1][1].append(f'<a href="{href}">{label}</a>')
    out = []
    for head, links in cols:
        out.append(f'<div><span>{head}</span>' + "".join(links) + "</div>")
    return "\n".join(out)


PAGE = """<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<meta name="description" content="{desc}">
<link rel="icon" href="favicon.svg" type="image/svg+xml">
<link rel="preload" href="vendor/fonts/figtree.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="vendor/fonts/sora.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="style.css">
<script src="vendor/motion.js" defer></script>
<script src="vendor/lottie.js" defer></script>
<script type="module" src="app.js"></script>
</head><body data-page="{page}">
<header class="topbar">
  <a class="brand" href="index.html">{markicon}<span>Grasm</span></a>
  <span class="ver">v{version}</span>
  <nav>
    <a href="different.html">Why Grasm</a>
    <a href="start.html">Docs</a>
    <a href="security.html">Security</a>
    <a href="achieved.html">Scorecard</a>
    <a href="playground.html">Playground</a>
  </nav>
  <a class="cta" href="start.html">Get started</a>
</header>
<div class="frame">
<nav class="rail">
{rail}
</nav>
<main>
{body}
</main>
</div>
<footer class="sitefoot">
  <div class="inner">
    <div class="cols">
      <div class="brandcol">
        <a class="brand" href="index.html">{markicon}<span>Grasm</span></a>
        <span>Four engines, one answer.</span>
      </div>
      <div class="links">
{foot}
      </div>
    </div>
    <p>Counts here are computed from the repository as the page is written;
    measurements are read from the document that recorded them.</p>
  </div>
</footer>
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
            markicon=mark(20, 6), version=known.get("version", ""),
            rail=rail(href), foot=foot(), body=fill(body)
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

    # A figure with no date is how a September number sat on an October page.
    # Both writers record one now, so one arriving without a date means a
    # writer regressed rather than that the date is unknowable.
    record = os.path.join(OUT, "facts.json")
    if args.check and os.path.exists(record):
        taken = json.load(open(record)).get("taken", {})
        undated = [k for k in MEASURED if k not in taken]
        if undated:
            print("site/facts.json states a figure with no date:")
            print("  " + " ".join(undated))
            print("the pages say which run wrote each figure, so a figure")
            print("without one cannot be stated honestly. site_facts() in")
            print("scripts/allgates.sh records the date; it has stopped.")
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
