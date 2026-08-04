# Editor support

## VS Code

The extension in `vscode/` provides syntax highlighting and connects to the
language server built into the compiler (`grasm lsp`).

Install for local development:

```
cd editors/vscode
npm install vscode-languageclient
code --install-extension .    # or: symlink into ~/.vscode/extensions
```

Point `grasm.serverPath` at your built binary if `grasm` is not on `PATH`.

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
