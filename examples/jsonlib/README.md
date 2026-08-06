# A JSON library, written in Grasm

Two packages: `json/` parses and renders, `app/` uses it. Run it:

```
grasm deps examples/jsonlib/app
grasm run  examples/jsonlib/app/src/main.gr
```

This exists because the only way left to find what a language cannot say is
to write something real in it. Roughly 250 lines, using recursive sum types,
`Map[str, Json]` values that are themselves the recursive type, error unions
carrying position information, an `inout` cursor threaded through the whole
parser, `fold`, `entries`, and a function passed by name to `map`.

`capabilities = []` on the library is the interesting line. Parsing is pure
computation, and that claim is checked against the source rather than
believed — add a `read_file` to `json/src/main.gr` and the **application**
stops building.

## What writing it found

Two pieces of friction, recorded because they are the whole point of the
exercise:

- A literal `{` in a string has to be written `\{`, since braces open an
  interpolation. Unavoidable for a JSON library, and the error message says
  exactly what to write.
- Typing `!x` for negation gives "expected an expression but found `!`",
  which does not mention that the word is `not`. It does now.

And one thing that used to be friction and no longer is: an object holds a
`Map[str, Json]`. The older `projects/intermediate/01_json.gr` keeps parallel
`keys` and `vals` lists instead, because a map whose value type is the
recursive type it lives in did not use to work. Rendering is deterministic
without sorting anything, because map iteration is already in key order.
