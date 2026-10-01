# json — the JSON parser, as a package

Project 13 wrote this parser. Project 34 needed it and, at first, carried a
copy; project 35 needed it too, and two copies is where a shared library stops
being a nicety.

It is a package rather than a sibling file because Grasm resolves `import json`
as a sibling of the importing file, so a module cannot be shared across
directories without one. A `grasm.toml` with a path dependency is what makes
`import json` mean *the same file* from two different projects.

```toml
[dependencies.json]
path = "../../packages/json"
version = "^1.0.0"
```

## What it exports, and what it does not

`parse_json`, `to_text`, `equal`, `escape_json`, `number_text`, and the `Json`
type. The scanner state, the character cursor and the sub-parsers stay private:
nothing outside needs to know that a recursive descent parser is what is behind
the door, and a module that exports its internals cannot change them.

## Reading without a tree

`reader`, `open_object`, `next_key`, `open_list`, `next_item`, `read_str`,
`read_int`, `read_num`, `read_bool`, `read_null`, `peek_kind`, `skip` and
`finish` read a document one value at a time from a cursor, so a program can
decode straight into its own types instead of building a `ref Json` tree and
walking it. They use the same scanners as `parse` and refuse what it refuses.
`json-demo` shows them, malformed input included.

## capabilities = []

A parser reads text and answers with a value. It opens no files, dials nothing
and starts no processes, so it declares nothing — and a consumer that imports it
cannot acquire a capability by doing so. That is the property the supply-chain
benchmark exists to check, stated here where it is cheap to hold to.
