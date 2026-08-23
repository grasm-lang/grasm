# 36 — Live Markdown Editor  *(phase 2, project 3)*

**Tests:** a second shared package, a compiler used by two routes, and what
happens when the browser cannot have it.
**Result:** works, with the browser half redesigned mid-project — see below.

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
the way `lib_json` was in project 5. `/render` compiles without storing;
`/save` compiles *and* stores. The harness checks they agree rather than
asserting it:

```
--- preview and publish agree ---
  identical html: true
  and the document was stored only by the second: true
```

## The design changed while it was being written

The plan said this was a project where "the browser half needs Grasm rather
than JavaScript" — the page would fetch `main.gr` and run it in wasm, exactly as
project 33 does for its validation rules.

**It cannot.** `grasm_run_with_stdin` takes one source string, and
`wasm32-unknown-unknown` has no filesystem, so a program with an `import`
cannot be handed to the wasm engine at all. Project 33 worked because it is a
single file. This one imports `lib_markdown`, which is the whole point of it.

That is recorded as **F33** — it is much larger than this project, because every
Grasm program worth writing has imports and the fourth engine can run none of
them. The browser showcase works only because all 31 phase-1 programs happen to
be single files, which was never a decision anyone made.

So the preview is a round trip to `/render`, debounced. This is not a workaround
dressed up as a design: the preview is then compiled not by the same *compiler*
as the published page but by the same *process*, and there is nothing left for
the two to disagree about. What it costs is a request per keystroke, and the
page says so where a reader will see it.

The honest way to get the browser back is `grasm build --source` — one `.gr`
file with the import graph inlined. The loader already resolves and holds every
file, so it is a pass over a structure that exists.

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
