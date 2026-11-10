# The Lunar compiler, in Lunar

A port of the Rust compiler (`crates/`) to Lunar, one stage at a time. It is
a Lunar workspace, built with `lunar build` from this directory. The goal is
a fully self-hosted compiler: `lunarc`, built from this workspace by the
Rust `lunar`, rebuilds this workspace into an identical compiler.

| Crater       | Port of            | State |
|--------------|--------------------|-------|
| `ln-lexer`   | `crates/ln-lexer`  | complete |
| `ln-ast`     | `crates/ln-ast`    | the syntax tree, in arenas (`core::arena`): every `Box` is an `Id`, every `Vec` a `List`, names and strings are interned |
| `ln-parser`  | `crates/ln-parser` and `crates/ln-parse-kit` | complete for source files; macro expansion parsing (`parse_expansion`, `parse_fragment`) is not ported yet |
| `ln-sema`    | `crates/ln-sema` and `crates/ln-diag` | data model done (`ty`, `defs`, `thir`, `program`, `diag`, `codes`); passes in progress (see below) |
| `ln-codegen` | `crates/ln-codegen` | in progress (LLVM through its C API, `#[link(name = "LLVM-C")]`) |
| `ln-driver`  | (test driver, then `lunarc`) | `ln-driver FILE...` prints tokens, `ln-driver --parse FILE...` the syntax tree and syntax errors |

## The port of the rest

| Part | Rust source | Lunar file(s) | State |
|------|-------------|---------------|-------|
| front end | `loader.rs`, `expand.rs`, `collect.rs`, `resolve.rs`, `lower.rs`, parser expansion parsing | `ln-sema/src/{sema,loader,expand,collect,resolve,lower}.ln` | in progress |
| traits, layout | `traits.rs`, `layout.rs` | `ln-sema/src/{traits,layout}.ln` | in progress |
| borrow checker | `borrowck.rs` | `ln-sema/src/borrowck.ln` | in progress |
| type checker | `typeck/*.rs` | `ln-sema/src/typeck*.ln` | after the front end |
| code generation | `crates/ln-codegen` | `ln-codegen/src/*.ln` | in progress |
| driver | `crates/ln-driver` (build path only) | `ln-driver/src/main.ln` (`lunarc`) | after the passes |

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
  exercises error recovery.

## Differences from the Rust parser

* The cursor's arrays only grow. A format string's holes are lexed onto the
  end of the token array and parsed with the cursor's limit moved there;
  their errors are marked dead rather than removed.
* Float literals keep their text (the span) rather than a parsed value.
* Integer literals hold up to 128 bits in two `u64` words.
