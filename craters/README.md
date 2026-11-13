# The Lunar compiler, in Lunar

The Rust compiler (`crates/`) ported to Lunar. It is a Lunar workspace,
built with `lunar build` from this directory, and it is self-hosted: the
compiler built from it by the Rust `lunar` compiles it again into a
compiler that compiles it into the very same executable.

## Building and bootstrapping

The code generator calls LLVM's C API, so linking needs the directory of
LLVM's shared library in `$LUNAR_LIB_PATH` (`LLVM-C.lib` on Windows,
`libLLVM` elsewhere); on Windows `LLVM-C.dll` must also be on `PATH` at run
time.

```sh
cd craters
export LUNAR_LIB_PATH="C:/Program Files/LLVM/lib"   # or `llvm-config --libdir`
lunar build --release                                # stage 1, by the Rust compiler
mkdir stage2 stage3
target/release/lunar build -p lunar -o stage2/lunar  # stage 2, by stage 1
stage2/lunar build -p lunar -o stage3/lunar          # stage 3, by stage 2
cmp stage2/lunar stage3/lunar                        # identical
```

(Both stages get the same file name because on macOS the linker's ad hoc
code signature records it.)

The executable, `target/release/lunar`, is a drop-in replacement for the
Rust `lunar`: the same commands (`new`, `init`, `check`, `build`, `run`,
`test`, `add`, `remove`, `update`, `clean`, `emit-llvm`, `explain`, `lsp`), options, help and error texts, status lines, exit codes
and diagnostics. Diagnostics are drawn by `ln-ariadne` exactly as the Rust
compiler draws them with ariadne (`ln-sema/src/render.ln` is the glue).
When stderr is a terminal and `NO_COLOR` is not set they are in color, and
the quoted code is syntax highlighted from `ln-lexer`'s tokens
(`ln-sema/src/highlight.ln`).

The Rust integration tests run against it with `LUNAR_BIN`:

```sh
LUNAR_BIN=$PWD/target/release/lunar cargo test -p ln-driver --test lang --test project --test hover --test lsp_session
```

### Where it finds the standard library

The Rust compiler embeds `std/` and `docs/errors/`; this one reads them at
run time, from the first of:

1. `$LUNAR_STD_DIR` (the standard library only: `core/` and `std/`);
2. an installation: `<prefix>/lib/lunar/std` and
   `<prefix>/lib/lunar/docs/errors`, for the executable
   `<prefix>/bin/lunar`;
3. a source checkout: the nearest directory above the executable with
   `std/core/lib.ln`, and its `std/` and `docs/errors/`. So
   `craters/target/release/lunar` uses the repository's.

To install it, copy the files into that layout:

```
<prefix>/bin/lunar(.exe)
<prefix>/lib/lunar/std/{core,std}/...       # this repository's std/
<prefix>/lib/lunar/docs/errors/<scope>/<name>.md
```

Without a standard library `lunar` reports `err(build::io): could not find
the standard library`; without `docs/errors`, `lunar explain` says so and
failed builds show no `lunar explain` hint.

| Crater       | Port of            | State |
|--------------|--------------------|-------|
| `ln-lexer`   | `crates/ln-lexer`  | complete |
| `ln-ast`     | `crates/ln-ast`    | the syntax tree, in arenas (`core::arena`): every `Box` is an `Id`, every `Vec` a `List`, names and strings are interned |
| `ln-parser`  | `crates/ln-parser` and `crates/ln-parse-kit` | complete: source files (`parse_file`, and `parse_file_into` a shared `Ast` with global offsets), macro expansions (`parse_expansion`, `parse_fragment`), hygiene contexts on identifiers and tokens |
| `ln-ariadne` | the `ariadne` crate (0.6) | complete: sources, labels (color, order, priority), reports with notes and helps, the whole `Config` and the layout algorithm, both character sets; ariadne's tests. Beyond ariadne: a `Theme` for every part of a report, and per-byte source styles (`Sources::add_styled`, `add_highlighted` with a `Highlighter`). Character widths approximate `unicode-width` (common wide and zero-width ranges) |
| `ln-sema`    | `crates/ln-sema` and `crates/ln-diag` | complete (see below) |
| `ln-codegen` | `crates/ln-codegen` | complete (LLVM through its C API, `llvm.ln`) |
| `ln-toml`    | the `toml` and `toml_edit` crates | what manifests and lockfiles need: the whole TOML syntax, every key and value with its place in the text; `edit` sets and removes keys in place, keeping comments and layout as `toml_edit` does |
| `ln-semver`  | the `semver` crate (1.0) | versions, requirements (`^`, `~`, `=`, `<`, `>`, wildcards, several comparators), matching with its pre-release rules, its display and error messages |
| `ln-json`    | what the language server gets from `serde_json` | JSON documents as arenas of linked nodes: strict parsing (escapes, `\u` surrogate pairs, numbers kept as written), building, compact writing with serde_json's escapes; tests in `src/tests.ln` |
| `lunar`      | `crates/ln-driver` | the `lunar` command (above): `cli.ln` (clap's parsing, help and errors), `commands.ln` (`main.rs`), `project.ln` and `git.ln` (`project.rs`), `compile.ln` (`lib.rs`), `explain.ln`, `link.ln`, `testing.ln`, `sysroot.ln`, and the language server, `lsp.ln`, `ide.ln`, `complete.ln`, `refs.ln` and `symbols.ln` (below). Hidden modes for `selfhost.rs`: `--lex FILE...` prints tokens, `--parse FILE...` the syntax tree and syntax errors, `--check FILE` the diagnostics one per line (`LN_EXPANSIONS=1` also prints every macro expansion), `--render [--color] FILE...` the diagnostics drawn as `lunar check` draws them, `--craters DIR` the craters a package build loads |

### The language server

`lunar lsp [--stdio]` (`lunar/src/lsp.ln` for the protocol; the queries in
`ide.ln`, `complete.ln`, `refs.ln` and `symbols.ln`; `ln_ast::pretty` for
declarations; `ln_sema::query` for the methods of a type):
`lsp-server`'s framing and lifecycle, full text synchronization, and

* diagnostics as you type (for std itself too, when a file of a std source
  tree is open), with quick fixes (`textDocument/codeAction`) from the exact
  suggestions diagnostics carry (`Diags::suggest`);
* hover with the Rust server's text, and go to definition;
* completion (`.` and `:` trigger it): locals in scope, items in scope and
  through paths, fields and methods by the receiver's type (inherent and
  trait methods, autoslicing), struct literal fields, `use` paths, keywords
  by place, snippets for calls, each with its declaration and docs
  (`completionItem/resolve` for long lists). It answers from the current
  text and the crate's last analysis, without waiting for a new one: the
  receiver's type is the expression the analysis typed at the same place,
  else followed along the tokens (locals, paths, fields, method results,
  indexing, `?`); see `complete.ln` for what is approximate;
* signature help, with parameters as written (`&mut +T`, `kept`);
* references and document highlights, and rename (with `prepareRename`):
  locals, items, fields, methods (a trait method with its implementations)
  and variants, across files and into the package's binary and tests for a
  library; names in macro arguments count where they are written
  (`Program::origins`); the standard library and keywords cannot be
  renamed;
* semantic tokens (full, delta and range), document symbols, workspace
  symbols, and inlay hints (`let` types, parameter names).

Open documents reach the loader as `Craters::overlay`. The standard library
comes from the sysroot, so definitions in it are `file:` locations;
`lunar-std:` URIs and `lunar/stdSource` still work. It keeps the analyses
of the four crates used last and analyzes others again when asked about
them; after changes it waits for a moment without input before analyzing
again. The session scripts in `tests/lsp/` (at the root of the repository)
test every feature: `lunar test -p lunar` replays them in process. The LSP
tests in `crates/ln-driver/tests` (`lang`, `project`, `hover`,
`lsp_session`) run against it with `LUNAR_BIN`; `lsp_session` compares its
answers with the Rust server's.

## The passes

| Part | Rust source | Lunar file(s) | State |
|------|-------------|---------------|-------|
| front end | `loader.rs`, `expand.rs`, `collect.rs`, `resolve.rs`, `lower.rs`, parser expansion parsing | `ln-sema/src/{sema,loader,expand,collect,resolve,lower}.ln` | done; same diagnostics as Rust on tests/ui, tests/run, examples |
| traits, layout | `traits.rs`, `layout.rs` | `ln-sema/src/{traits,layout}.ln` | done (unit checks in `tests.ln`) |
| borrow checker | `borrowck.rs` | `ln-sema/src/borrowck.ln` | done (unit checks on hand-built THIR in `tests.ln`) |
| type checker | `typeck/*.rs` | `ln-sema/src/typeck/{mod,expr,call,pat,closure}.ln` | done; Rust's `FnCtx` is the `Sema::fcx` field |
| code generation | `crates/ln-codegen` | `ln-codegen/src/{lib,func,ops,init,llvm}.ln` | done; the state of the function being lowered (Rust's `FnCg`) lives in `Cg` |
| driver | `crates/ln-driver` (all but `lsp.rs`, `ide.rs`) | `lunar/src/*.ln` | done; std and `docs/errors` are read from the sysroot, not embedded |
| language server | `lsp.rs`, `ide.rs`, `ln-ast/src/pretty.rs` | `lunar/src/{lsp,ide,complete,refs,symbols}.ln`, `ln-ast/src/pretty.ln`, `ln-sema/src/query.ln` | done, and beyond the Rust server: the same answers where both answer (`tests/lsp_session.rs`); completion, signature help, references, rename, semantic tokens, symbols, inlay hints and quick fixes (`tests/lsp/`) |

### Data model

* Types are `Ty` handles into the `Types` interner (`ty.ln`): equal types
  are equal handles; type lists (`TyList`) are interned too. Rebuild types
  with `Types::fold` and a `Folder`; substitute with `Types::subst`.
* Definitions (`defs.ln`) live in `Program::defs`; everything in them is
  Light, with lists in the program's arenas. Module bindings and impl
  methods are program-wide maps.
* THIR nodes (`thir.ln`) live in `Program::thir`'s arenas, linked by `Id`.
* Spans are global offsets into `Program::sources` (every file's text, end
  to end); macro expansions are files too.
* Diagnostics (`diag.ln`) are built with `Diags::error` / `label` /
  `note` / `help` / `finish`; codes are functions (`codes::ty::MISMATCH()`).
  Building one only grows `Diags` (`&mut +self`), so a pass holding a
  `p: &mut +Program` can report errors into `p.diags`.
* Trait queries, layouts and the borrow checker take the program as
  `&mut +Program` (substituting interns types): `p.is_dull(t, env)`,
  `p.layout_of(t)`, `borrowck::check(p, body, env)`.
  `borrowck::lint(p, body, &mut lints)` writes warnings into a separate
  `Diags` (copy them over with `Diags::push_from`).

### Writing Lunar for the port

* Structs cannot hold references: state a Rust struct borrows is passed to
  methods as parameters, or owned.
* Array items, map keys and values must be Light (own no array or string).
* What a returned value's views point into is part of the signature
  (`docs/reference.md` §4): a method's result points into `self`, any
  other function's into every parameter a view could come from, unless a
  `from` clause says otherwise. Say it when callers need the other
  arguments free: `fn text(&self, src: str): str from src`. A result that
  holds views only of static data, such as a `ProjectError` (whose `Code`
  is two string literals) or a `Sema` built from copies, is `from static`:
  `fn check_req(alias: str, req: str, path: str, lo: u32, hi: u32):
  Result::<(), ProjectError> from static`. Closures are still inferred
  from their bodies; calls through function pointers count as pointing
  into every argument.
* Constants cannot hold structs.
* `[T; ..+]` only grows; use `[T; -..+]` to `pop`/`truncate`/`clear`.
* A struct holding a `Sema` (or anything with a `str` in it) counts as
  holding views: moving one out of a field and back trips the borrow
  checker, so keep it in place and query it there.
* A `&mut` method may shrink what it is given, so it ends views into it;
  `&mut +self` methods only grow. Parameters whose views a function stores
  into an in-out argument are `kept`.

## Checked against the Rust compiler

`crates/ln-driver/tests/selfhost.rs` builds this workspace and runs it over
every `.ln` file in the repository:

* `lexer_matches_rust_lexer`: the same tokens, lexical errors and conflict
  markers;
* `parser_matches_rust_parser`: the same syntax tree (`ln_ast::dump`, the
  same text as `crates/ln-ast/src/dump.rs`) and the same syntax errors (code,
  span and message);
* `parser_matches_rust_parser_on_broken_programs`: the same, for every
  program cut off a third and two thirds of the way through, which
  exercises error recovery;
* `diagnostics_render_as_rust_renders_them`: on every file in `tests/ui`,
  `--render` prints byte for byte what `lunar check` prints without color
  (before its summary line): the same diagnostics, drawn the same;
* `compiler_builds_itself`: the bootstrap above; stages 2 and 3 must have
  the same LLVM IR and the same bytes (on Windows the link is reproducible
  through `/Brepro`; on macOS the linker's ad hoc code signature names the
  executable, so both stages are linked to files of the same name).

Programs compiled by the Lunar compiler pass every run test
(`tests/run`, `examples`), and `--check` reports the Rust compiler's
codes at the same positions on `tests/ui`. With `LUNAR_BIN` set to it,
the command-line and language server tests (`tests/lang.rs`,
`tests/project.rs`, `tests/hover.rs`, `tests/lsp_session.rs`) pass.

## Differences from the Rust driver

* `std/` and `docs/errors/` come from the sysroot (above), not from the
  executable.
* On Windows, linking falls back from `clang` to a `link.exe` on `PATH`;
  the Rust driver also finds an installed MSVC through the registry.
* I/O errors are worded by `std` (`not found (os error 2)`), not by the
  operating system.
* Manifest errors that Rust's serde reports with its own spans (an unknown
  key, a value of the wrong type) point at the key or value instead.

## Differences from the Rust parser

* The cursor's arrays only grow. A format string's holes are lexed onto the
  end of the token array and parsed with the cursor's limit moved there;
  their errors are marked dead rather than removed.
* Float literals keep their text (the span) rather than a parsed value.
* Integer literals hold up to 128 bits in two `u64` words.
