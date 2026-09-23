# Aura — VS Code extension

Syntax highlighting and language-server support for `.aura` files.

## Features

- TextMate syntax highlighting (keywords, types, literals, comments, strings)
- Diagnostics as you type (full pipeline: parse → resolve → type-check)
- Hover: inferred type of the expression under the cursor
- Go to definition (file-scope items)
- Document outline / breadcrumbs (`Ctrl+Shift+O`)
- Document formatting (`Shift+Alt+F`) via `aura fmt`'s engine

## Requirements

The `aura` compiler binary on your `PATH` (or set `aura.serverPath` to an
absolute path). Build it from the repo root:

    cargo build --release -p aura-cli

The extension launches `aura lsp` and speaks LSP over stdio.

## Install (development)

From this directory:

    npm install        # fetches vscode-languageclient
    npx vsce package   # produces aura-lang-<ver>.vsix
    code --install-extension aura-lang-<ver>.vsix

Or symlink this folder into `%USERPROFILE%\.vscode\extensions\` and reload VS Code
(run `npm install` first so `vscode-languageclient` resolves).

## Settings

| Setting             | Default | Description                                  |
| ------------------- | ------- | -------------------------------------------- |
| `aura.serverPath`   | `aura`  | Path to the aura binary                      |
| `aura.trace.server` | `off`   | LSP message tracing (`off`/`messages`/`verbose`) |
