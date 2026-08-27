# 36 — Live Markdown Editor  *(phase 2, project 3)*

**Tests:** a second shared package, a compiler used by two routes, and a
browser running a multi-module Grasm program.
**Result:** works. The browser half was redesigned twice — see below; the
second redesign was to fix the language, not the project.

```
grasm run main.gr < /dev/null      # the comparison the harness makes
echo '# hi' | grasm run main.gr    # one document in, HTML out
grasm run main.gr -- serve         # then serve index.html over http
```

## The claim

An editor whose preview is rendered by a different implementation from the one
that publishes is an editor that lies: the writer sees one document and the
reader gets another, and the difference only appears on the constructs where two
markdown implementations disagree — which is most of the interesting ones.

So there is one compiler. `lib_markdown` is project 12's, extracted as a package
the way `json` was in project 5. `/render` compiles without storing;
`/save` compiles *and* stores. The harness checks they agree rather than
asserting it:

```
--- preview and publish agree ---
  identical html: true
  and the document was stored only by the second: true
```

## The design changed twice while it was being written

The plan said this was a project where "the browser half needs Grasm rather
than JavaScript" — the page would fetch the source and run it in wasm, exactly
as project 33 does for its validation rules.

**It could not.** `grasm_run_with_stdin` takes one source string and
`wasm32-unknown-unknown` has no filesystem, so a program with an `import` could
not be handed to the wasm engine at all. Project 33 worked because it is a
single file. This one imports `lib_markdown`, which is the whole point of it.

That was **F33**, and it was much larger than this project: every Grasm program
worth writing has imports, and the fourth engine could run none of them. The
browser showcase worked only because all 31 phase-1 programs happen to be single
files, which was never a decision anyone made.

So the second design asked the server to render on every keystroke. Honest, and
worse.

The third is the first one, because F33 is fixed. `grasm build --source` writes
a **source bundle** — the whole program as one `.gr`, modules laid end to end
behind `//!module` lines — and the page fetches that. The preview is compiled by
the server's own compiler, in the browser, and the HTML it produces was diffed
against the HTML the server publishes for the same document:

```
BROWSER PREVIEW == SERVER PUBLISH, byte for byte
```

Publishing is now the only round trip, and the page reports whether the server
agreed with what the writer was shown.

## The wire format

A document travels as one line, `\n` written as two characters, because
`input()` reads one line. It is deliberately the dullest thing that works, and
the harness checks five documents survive the round trip — including one that is
empty, one that is only newlines, and one containing a backslash.

## Verified live

```
$ curl -s -XPOST localhost:8977/render --data-binary @doc.md
<h1>Live</h1><p><em>one</em> and <strong>two</strong></p><ul><li>a</li><li>b</li></ul>
$ curl -s -XPOST localhost:8977/save --data-binary @doc.md    # byte-identical
$ curl -s localhost:8977/source                               # the stored markdown
$ curl -s localhost:8977/                                     # the published page
```

`render` and `save` were diffed, not eyeballed: identical.
