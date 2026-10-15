# Editor support

## The grammar is generated

`vscode/syntaxes/grasm.tmLanguage.json` is written by
`scripts/grammar.py` from the compiler itself -- keywords from
`crates/grasm_syntax/src/token.rs`, builtins from
`crates/grasm_sema/src/builtins.rs`, primitive types from the `prim` match in
`crates/grasm_sema/src/check.rs`. Edit the generator, not the JSON, and run:

```
python3 scripts/grammar.py
```

The `grammar` gate runs it with `--check` and fails the build when the
committed file is not what it would write, so a keyword added to the language
cannot be missing from the highlighting. A new keyword also has to be given a
scope in `SCOPES`, which is an error rather than a default: whether a word
reads as control flow or as a modifier is a decision, not something to infer.

This is here because the hand-maintained version drifted badly -- `interface`,
`impl`, `unsafe` and `ref` unhighlighted, `bytes` and `range` missing, 41 of
52 builtins absent, two names listed that are not builtins, and `"""` raw
strings unhandled, which left a raw block's contents highlighted as live code
and interpolation.

## VS Code

The extension in `vscode/` provides syntax highlighting, connects to the
language server built into the compiler (`grasm lsp`), and registers a debugger
that speaks to `grasm dap`.

Install for local development:

```
cd editors/vscode
npm install vscode-languageclient
code --install-extension .    # or: symlink into ~/.vscode/extensions
```

Point `grasm.serverPath` at your built binary if `grasm` is not on `PATH`.

### Debugging

`F5` on a `.gr` file runs it with breakpoints, stepping and variables. No
`launch.json` needed for that; one is needed to replay:

```json
{
  "type": "grasm",
  "request": "launch",
  "name": "Replay a recording",
  "program": "${workspaceFolder}/src/main.gr",
  "journal": "${workspaceFolder}/production.jrnl"
}
```

With `journal` set, every effect is answered from the recording and nothing
reaches the world — not the reads and not the writes. Stop on the line after
`now_millis()` and the variable holds the time the failure happened, not the
time you looked. See [../docs/replay.md](../docs/replay.md).

Any DAP-capable editor can do the same: the adapter is `grasm dap` on stdio,
and `journal` is a field in the launch arguments.

## Any LSP-capable editor

The server speaks LSP over stdio — no editor-specific code required.

- **Neovim**: `vim.lsp.start({ name = 'grasm', cmd = { 'grasm', 'lsp' }, filetypes = { 'grasm' } })`
- **Helix**: add to `languages.toml`:
  ```toml
  [[language]]
  name = "grasm"
  file-types = ["gr"]
  language-servers = ["grasm-lsp"]

  [language-server.grasm-lsp]
  command = "grasm"
  args = ["lsp"]
  ```
- **Emacs (eglot)**: `(add-to-list 'eglot-server-programs '(grasm-mode . ("grasm" "lsp")))`

## Provided features

| Feature | Notes |
|---|---|
| Diagnostics | On open, change, and save — the same checker `grasm check` uses |
| Hover | Function signatures, type declarations, variants, builtin docs |
| Go to definition | Functions, types, effects, sum-type variants |
| Completion | File declarations, variants, keywords, builtins |
| Document symbols | Outline of every declaration |
| Debugging | Breakpoints, step in/over/out, variables, stack traces (`grasm dap`) |
| Replay debugging | `"journal": "<file>"` steps through a recorded run instead of a live one |
