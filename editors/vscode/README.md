# Lunar for VS Code

Language support for [Lunar](https://github.com/lunarln/lunar):

* Syntax highlighting for `.ln` files (and ```` ```lunar ```` blocks in Markdown),
  refined by the language server's semantic tokens: types, functions,
  methods, macros, locals, parameters, fields, variants, traits and modules,
  with the standard library's names and what is `mut` (underlined) told
  apart.
* Diagnostics as you type, with the scoped codes (`err(borrow::shrink_while_borrowed)`).
  Run `lunar explain <code>` for a longer explanation of any of them. Quick
  fixes apply the exact fixes many of them suggest (`&mut x`, `_unused`,
  `let mut`, `T: Light`, `.as_str()`, ...).
* Completion: locals, items in scope and through paths (`std::fs::`,
  `Enum::`, `Type::`), fields and methods after `.`, struct literal fields,
  `use` paths and keywords, with signatures, docs and call snippets.
* Signature help, with parameter modes as written (`&mut +T`, `kept`).
* Hover: signatures and doc comments of items, types of locals, fields and
  expressions.
* Go to definition, including into the standard library; find references,
  highlights and rename (across the files of a package).
* An outline of each file, workspace symbol search, and inlay hints (the
  types of `let`s, parameter names at calls).

The language features come from `lunar lsp`, so the `lunar` executable must
be on your `PATH` (or set `lunar.server.path`). Use the compiler built from
`craters/` (see `craters/README.md`): the Rust compiler in `crates/` serves
only diagnostics, hover and go to definition.

## Building the extension

```sh
cd editors/vscode
npm install
npx @vscode/vsce package
code --install-extension lunar-0.1.0.vsix
```
