# The Lunar compiler

The compiler, package manager and language server, written in Lunar. It
is a Lunar workspace, built with `lunar build` from this directory, and it
is self-hosted: a released compiler builds it, and the compiler that makes
builds it again into the very same executable.

## Building and bootstrapping

The code generator calls LLVM's C API, so linking needs the directory of
LLVM's shared library in `$LUNAR_LIB_PATH` (`LLVM-C.lib` on Windows,
`libLLVM` elsewhere); on Windows `LLVM-C.dll` must also be on `PATH` at run
time.

```sh
cd craters
export LUNAR_LIB_PATH="C:/Program Files/LLVM/lib"   # or `llvm-config --libdir`
lunar build --release                                # with a released lunar
target/release/lunar --version
```

The released compiler is the one in [stage0.txt](stage0.txt) (or a newer
one). [`tools/bootstrap.sh`](../tools/bootstrap.sh) does the whole
bootstrap, as CI and releases do:

```sh
tools/bootstrap.sh path/to/stage0/bin/lunar out
# stage 1: craters/ built by stage 0, with stage 0's own std
# stage 2: built by stage 1, with the repository's std/
# stage 3: built by stage 2; stages 2 and 3 must be identical
```

Each stage is linked to a file of the same name in its own directory,
because on macOS the linker's ad hoc code signature records it. The
stages are the same bytes whatever they are built with: the debug
information has no timestamps or addresses, source paths are recorded
relative to `craters/` (std's under its own directory), and on Windows the
executable names its PDB without a directory (`/PDBALTPATH:%_PDB%`) and is
linked with `/Brepro`.

What `craters/` may use (only what stage 0 supports) and how to move
stage 0 forward is in [CONTRIBUTING.md](../CONTRIBUTING.md).

The executable has the commands `new`, `init`, `check`, `build`, `run`,
`test`, `add`, `remove`, `update`, `clean`, `emit-llvm`, `explain`, `fmt`
(the formatter, [docs/style.md](../docs/style.md)) and `lsp`. Diagnostics are drawn by `ln-ariadne` (`ln-sema/src/render.ln` is
the glue). When stderr is a terminal and `NO_COLOR` is not set they are in
color, and the quoted code is syntax highlighted from `ln-lexer`'s tokens
(`ln-sema/src/highlight.ln`).

### Tests

* `lunar test -p lunar`: the compiler's own tests, among them the
  language server sessions in `tests/lsp/` replayed in process
  (`lunar/src/ide_tests.ln`) and the debug information
  (`lunar/src/debuginfo_tests.ln`: the IR, a scripted lldb session when
  lldb starts, and the PDB through `llvm-pdbutil`). The other packages
  have unit tests too (`lunar test -p ln-json`, ...).
* `LUNAR_BIN=$PWD/target/release/lunar lunar test -p lunar-tests`:
  [lunar-tests](lunar-tests/src/lib.ln) runs the executable through its
  command line and language server, on `tests/ui`, `tests/run`,
  `tests/fatal`, `examples`, `docs/errors`, projects and workspaces,
  `tests/lsp` over `lunar lsp`, and broken programs. It uses
  `std::process::Command` to start `lunar` and talk to it.

### Where it finds the standard library

The standard library and the explanations of the diagnostic codes are not
embedded: they are read at run time, from the first of:

1. `$LUNAR_STD_DIR` (the standard library only: `core/` and `std/`);
2. an installation: `<prefix>/lib/lunar/std` and
   `<prefix>/lib/lunar/docs/errors`, for the executable
   `<prefix>/bin/lunar` (the layout of the release archives);
3. a source checkout: the nearest directory above the executable with
   `std/core/lib.ln`, and its `std/` and `docs/errors/`. So
   `craters/target/release/lunar` uses the repository's.

Without a standard library `lunar` reports `err(build::io): could not find
the standard library`; without `docs/errors`, `lunar explain` says so and
failed builds show no `lunar explain` hint.

| Crater       | Contents |
|--------------|----------|
| `ln-lexer`   | tokens and lexical errors, conflict markers |
| `ln-ast`     | the syntax tree, in arenas (`core::arena`): nodes are `Id`s, lists `List`s, names and strings are interned; `pretty` prints declarations |
| `ln-parser`  | source files (`parse_file`, and `parse_file_into` a shared `Ast` with global offsets), macro expansions (`parse_expansion`, `parse_fragment`), hygiene contexts on identifiers and tokens |
| `ln-ariadne` | a port of the `ariadne` crate (0.6): sources, labels (color, order, priority), reports with notes and helps, the whole `Config` and the layout algorithm, both character sets. Beyond ariadne: a `Theme` for every part of a report, and per-byte source styles (`Sources::add_styled`, `add_highlighted` with a `Highlighter`). Character widths approximate `unicode-width` (common wide and zero-width ranges) |
| `ln-sema`    | semantic analysis and diagnostics (see below) |
| `ln-codegen` | LLVM code generation through LLVM's C API (`llvm.ln`), and debug information (`debug.ln`) |
| `ln-toml`    | what manifests and lockfiles need: the whole TOML syntax, every key and value with its place in the text; `edit` sets and removes keys in place, keeping comments and layout |
| `ln-semver`  | versions and requirements (`^`, `~`, `=`, `<`, `>`, wildcards, several comparators), matching with Cargo's pre-release rules |
| `ln-json`    | JSON documents as arenas of linked nodes: strict parsing (escapes, `\u` surrogate pairs, numbers kept as written), building, compact writing |
| `ln-fmt`     | the formatter, in rustfmt's style: `format_source(text)` gives the text formatted ([docs/style.md](../docs/style.md)), or why not. The syntax tree is printed into layout documents laid out in 100 columns (`doc.ln`: Wadler's algorithm as Prettier has it, with rustfmt's width heuristics); comments, found between the tokens (`source.ln`), are put back by position. Every result is parsed again and must have the same tree and comments (`verify.ln`) |
| `lunar`      | the `lunar` command: `cli.ln` (parsing, help and errors), `commands.ln`, `project.ln` and `git.ln` (packages), `compile.ln`, `explain.ln`, `link.ln`, `testing.ln`, `sysroot.ln`, `fmt.ln` (`lunar fmt`), and the language server, `lsp.ln`, `ide.ln`, `complete.ln`, `refs.ln` and `symbols.ln` (below). Hidden modes, for looking at one stage of the front end: `--lex FILE...` prints tokens, `--parse FILE...` the syntax tree and syntax errors, `--check FILE` the diagnostics one per line (`LN_EXPANSIONS=1` also prints every macro expansion), `--render [--color] FILE...` the diagnostics drawn as `lunar check` draws them, `--craters DIR` the craters a package build loads |
| `lunar-tests` | the tests of the executable (above) |

### The language server

`lunar lsp [--stdio]` (`lunar/src/lsp.ln` for the protocol; the queries in
`ide.ln`, `complete.ln`, `refs.ln` and `symbols.ln`; `ln_ast::pretty` for
declarations; `ln_sema::query` for the methods of a type):
`lsp-server`'s framing and lifecycle, full text synchronization, and

* diagnostics as you type (for std itself too, when a file of a std source
  tree is open), with quick fixes (`textDocument/codeAction`) from the exact
  suggestions diagnostics carry (`Diags::suggest`);
* hover, and go to definition;
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
test every feature, in process (`lunar test -p lunar`) and over `lunar lsp`
(`lunar test -p lunar-tests`).

## The passes

| Part | File(s) |
|------|---------|
| front end: loading, macro expansion, collection, name resolution, lowering | `ln-sema/src/{sema,loader,expand,collect,resolve,lower}.ln` |
| traits, layout | `ln-sema/src/{traits,layout}.ln` (unit checks in `tests.ln`) |
| borrow checker | `ln-sema/src/borrowck.ln` (unit checks on hand-built THIR in `tests.ln`) |
| type checker | `ln-sema/src/typeck/{mod,expr,call,pat,closure}.ln`; the state of the function being checked is the `Sema::fcx` field |
| code generation | `ln-codegen/src/{lib,func,ops,init,llvm}.ln`; the state of the function being lowered lives in `Cg` |
| debug information | `ln-codegen/src/debug.ln`: CodeView / DWARF through LLVM's DIBuilder: functions, lines, variables in lexical blocks, types |
| driver | `lunar/src/*.ln` |
| language server | `lunar/src/{lsp,ide,complete,refs,symbols}.ln`, `ln-ast/src/pretty.ln`, `ln-sema/src/query.ln` |

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

### Writing the compiler in Lunar

* Use only what the stage 0 release supports (language and std):
  [CONTRIBUTING.md](../CONTRIBUTING.md).
* Structs cannot hold references: state a struct would borrow is passed to
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

## Notes

* On Windows, linking falls back from `clang` to a `link.exe` on `PATH`
  (as in a Developer PowerShell); it does not look for an installed MSVC
  through the registry.
* I/O errors are worded by `std` (`not found (os error 2)`), not by the
  operating system.
* The parser's arrays only grow. A format string's holes are lexed onto the
  end of the token array and parsed with the cursor's limit moved there;
  their errors are marked dead rather than removed.
* Float literals keep their text (the span) rather than a parsed value;
  integer literals hold up to 128 bits in two `u64` words.
