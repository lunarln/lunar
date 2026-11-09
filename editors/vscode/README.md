# Lunar for VS Code

Language support for [Lunar](https://github.com/WorriedArrow/lunar):

* Syntax highlighting for `.ln` files (and ```` ```lunar ```` blocks in Markdown).
* Diagnostics as you type, with the scoped codes (`err(borrow::shrink_while_borrowed)`).
  Run `lunar explain <code>` for a longer explanation of any of them.
* Hover: signatures and doc comments of items, types of locals, fields and
  expressions.
* Go to definition, including into the standard library.

The language features come from `lunar lsp`, so the `lunar` executable must
be on your `PATH` (or set `lunar.server.path`):

```sh
cargo install --path crates/ln-driver
```

## Building the extension

```sh
cd editors/vscode
npm install
npx @vscode/vsce package
code --install-extension lunar-0.1.0.vsix
```
