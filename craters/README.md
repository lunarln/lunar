# The Lunar compiler, in Lunar

A port of the Rust compiler (`crates/`) to Lunar, one stage at a time. It is
a Lunar workspace, built with `lunar build` from this directory.

| Crater      | Port of           | State |
|-------------|-------------------|-------|
| `ln-lexer`  | `crates/ln-lexer` | complete |
| `ln-ast`    | `crates/ln-ast`   | the syntax tree, in arenas (`core::arena`): every `Box` is an `Id`, every `Vec` a `List`, names and strings are interned |
| `ln-parser` | `crates/ln-parser` and `crates/ln-parse-kit` | complete for source files; macro expansion parsing (`parse_expansion`, `parse_fragment`) is not ported yet |
| `ln-driver` | (test driver)     | `ln-driver FILE...` prints tokens, `ln-driver --parse FILE...` the syntax tree and syntax errors |

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
