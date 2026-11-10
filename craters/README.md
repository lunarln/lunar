# The Lunar compiler, in Lunar

The Rust compiler (`crates/`) ported to Lunar. It is a Lunar workspace,
built with `lunar build` from this directory, and it is self-hosted: the
compiler built from it by the Rust `lunar` compiles it again into a
compiler that compiles it into the very same executable.

## Building and bootstrapping

The code generator calls LLVM's C API, so linking needs the directory of
LLVM's shared library in `$LUNAR_LIB_PATH` (`LLVM-C.lib` on Windows,
`libLLVM` elsewhere); on Windows `LLVM-C.dll` must also be on `PATH` at run
time. The compiler reads `core` and `std` from `$LUNAR_STD_DIR`.

```sh
cd craters
export LUNAR_LIB_PATH="C:/Program Files/LLVM/lib"   # or `llvm-config --libdir`
export LUNAR_STD_DIR=../std
lunar build --release                                # stage 1, by the Rust compiler
target/release/ln-driver build ln-driver -o stage2   # stage 2, by stage 1
./stage2 build ln-driver -o stage3                   # stage 3, by stage 2
cmp stage2 stage3                                    # identical
```

`ln-driver build [--release] [--test] [-o OUT] [--emit-llvm FILE] [PATH]`
compiles `PATH`, a `.ln` file or a package directory (default `.`), with
the libraries its `lunar.toml` names. Diagnostics are drawn by
`ln-ariadne` exactly as the Rust compiler draws them with ariadne
(`ln-sema/src/render.ln` is the glue). When stderr is a terminal and
`NO_COLOR` is not set they are in color, and the quoted code is syntax
highlighted from `ln-lexer`'s tokens (`ln-sema/src/highlight.ln`).

| Crater       | Port of            | State |
|--------------|--------------------|-------|
| `ln-lexer`   | `crates/ln-lexer`  | complete |
| `ln-ast`     | `crates/ln-ast`    | the syntax tree, in arenas (`core::arena`): every `Box` is an `Id`, every `Vec` a `List`, names and strings are interned |
| `ln-parser`  | `crates/ln-parser` and `crates/ln-parse-kit` | complete: source files (`parse_file`, and `parse_file_into` a shared `Ast` with global offsets), macro expansions (`parse_expansion`, `parse_fragment`), hygiene contexts on identifiers and tokens |
| `ln-ariadne` | the `ariadne` crate (0.6) | complete: sources, labels (color, order, priority), reports with notes and helps, the whole `Config` and the layout algorithm, both character sets; ariadne's tests. Beyond ariadne: a `Theme` for every part of a report, and per-byte source styles (`Sources::add_styled`, `add_highlighted` with a `Highlighter`). Character widths approximate `unicode-width` (common wide and zero-width ranges) |
| `ln-sema`    | `crates/ln-sema` and `crates/ln-diag` | complete (see below) |
| `ln-codegen` | `crates/ln-codegen` | complete (LLVM through its C API, `llvm.ln`) |
| `ln-driver`  | `crates/ln-driver` (the build path) | `ln-driver build` (above); also `ln-driver FILE...` prints tokens, `--parse FILE...` the syntax tree and syntax errors, `--check FILE` the diagnostics one per line (`LN_EXPANSIONS=1` also prints every macro expansion), `--render [--color] FILE...` the diagnostics drawn as `lunar check` draws them, `--craters DIR` the craters a package build loads |

## The passes

| Part | Rust source | Lunar file(s) | State |
|------|-------------|---------------|-------|
| front end | `loader.rs`, `expand.rs`, `collect.rs`, `resolve.rs`, `lower.rs`, parser expansion parsing | `ln-sema/src/{sema,loader,expand,collect,resolve,lower}.ln` | done; same diagnostics as Rust on tests/ui, tests/run, examples |
| traits, layout | `traits.rs`, `layout.rs` | `ln-sema/src/{traits,layout}.ln` | done (unit checks in `tests.ln`) |
| borrow checker | `borrowck.rs` | `ln-sema/src/borrowck.ln` | done (unit checks on hand-built THIR in `tests.ln`) |
| type checker | `typeck/*.rs` | `ln-sema/src/typeck/{mod,expr,call,pat,closure}.ln` | done; Rust's `FnCtx` is the `Sema::fcx` field |
| code generation | `crates/ln-codegen` | `ln-codegen/src/{lib,func,ops,init,llvm}.ln` | done; the state of the function being lowered (Rust's `FnCg`) lives in `Cg` |
| driver | `crates/ln-driver` (build path) | `ln-driver/src/{main,project,link}.ln` | done; std is read from disk, not embedded |

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
* Methods returning views of `&self` count as pointing into `self`; slice a
  `str` field through a free function instead (`tok_text(self.src, t)`).
* No bindings inside `|` patterns; write separate arms.
* Constants cannot hold structs.
* `[T; ..+]` only grows; use `[T; -..+]` to `pop`/`truncate`/`clear`.
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
  through `/Brepro`).

Programs compiled by the Lunar compiler pass every run test
(`tests/run`, `examples`), and `--check` reports the Rust compiler's
codes at the same positions on `tests/ui`.

## Differences from the Rust parser

* The cursor's arrays only grow. A format string's holes are lexed onto the
  end of the token array and parsed with the cursor's limit moved there;
  their errors are marked dead rather than removed.
* Float literals keep their text (the span) rather than a parsed value.
* Integer literals hold up to 128 bits in two `u64` words.
