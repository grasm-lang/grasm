# 12 — Markdown to HTML Compiler
**Tests:** multi-pass parsing, nested inline spans, escaping order. **Result:** works, all three engines agree.

The trap is doing it in one pass of replacements: replace `**` with `<strong>`
and you get `<strong>` twice and never `</strong>`; escape HTML after emitting
tags and you destroy your own output. So this is two passes with a defined
order — block structure, then inline spans — with escaping applied to text at
the moment it is emitted and never to markup.

Code spans win over emphasis, which is the whole point of them: `` `a < b && c` ``
comes out as `<code>a &lt; b &amp;&amp; c</code>`. Fences suppress markup
entirely. Unclosed `**`, `` ` `` and ``` ``` ``` are all handled — the first two
stay literal, the third is closed at end of document rather than emitting a
`<pre>` that never ends.

One bug of my own, found and fixed: `***both***` first produced
`<strong>*both</strong>*`, because a `want = 2` rule consumes two of three stars
and leaves the third. Three or more now means both tags.

## Findings
None in the language.
