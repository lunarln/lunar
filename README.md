# Lunar

[![CI](https://github.com/WorriedArrow/lunar/actions/workflows/ci.yml/badge.svg)](https://github.com/WorriedArrow/lunar/actions/workflows/ci.yml)

Lunar is a compiled systems language with Rust-like syntax and a
[Goose](https://github.com/aardappel/goose/wiki)-inspired memory model:
there is no heap and no garbage collector. Every owning array lives on its
own **data stack**, a large address-space reservation whose pages are
committed on demand, so arrays grow without ever moving and views into them
stay valid.

```lunar
fn len_squared::<T: Dull>(slice: [T; &]): usize {
    let len: usize = slice.len();
    return std::math::pow(len, 2);
}

fn main() {
    let mut arena: [i64; ..+] = [];
    for i in 0..200 {
        arena.push(i);
    }
    let slice: [i64; &] = arena[2..6];
    arena.push(201); // growing never moves elements
    println("fourth element of arena is STILL {slice[1]}");
}
```

## Highlights

* **Array kinds**: `[T; 4]` (inline), `[T; _]` (inferred length), `[T]`
  (sized once, then frozen), `[T; ..+]` (grow-only), `[T; -..+]` (grow and
  shrink), `[T; &]` / `[T; &mut]` (slices).
* **`!` parameters**: functions that may shrink a value take it as
  `mut x: !T` (`![i32; -..+]`, `!String`, or a struct containing one), and
  shrinking methods take `mut self: !Self`. Only the owner can make that
  conversion, and only while no live view points into it. The compiler
  checks this.
* **`Dull`**: a compiler-implemented trait for types without pointers or
  arrays. Only Dull types can be array items:
  `err(type::array_pointy_items)`.
* **Strings**: `str` is a UTF-8 view; `String` is owned UTF-8 text laid out
  like a `[u8; -..+]`. A string literal passed to a `FmtArgs` parameter is
  interpolated: `println("x = {x}")`.
* **Traits** like Rust's: generic traits, supertraits, default methods,
  associated functions, blanket impls, operator traits.
* **Hard `unsafe` rules**: an `unsafe fn` body gets no implicit unsafe
  context. Unsafe operations always need an `unsafe { }` block.
* **Scoped diagnostics** rendered with ariadne:
  `err(borrow::shrink_while_borrowed)`, `err(type::mismatch)`, ...

See [docs/reference.md](docs/reference.md) for the full language reference.

## Building

Requirements:

* Rust (stable, edition 2024)
* LLVM 20 (shared library)
  * Windows: the official installer from
    [llvm-project releases](https://github.com/llvm/llvm-project/releases).
    It is found in `C:\Program Files\LLVM` or via `LLVM_SYS_201_PREFIX`, and
    `LLVM-C.dll` must be on `PATH`.
  * Linux: `llvm-20-dev` (e.g. from [apt.llvm.org](https://apt.llvm.org)).
  * macOS: `brew install llvm@20`.
  * `LLVM_CONFIG` can point at a specific `llvm-config` on Unix.
* A system linker for the programs Lunar compiles: `clang` or the MSVC build
  tools on Windows, `cc` on Linux and macOS (override with `LUNAR_CC`).

```sh
cargo build --release
./target/release/lunar run examples/arrays.ln
```

## Usage

```text
lunar check <file.ln> [--target-os linux|macos|windows]
lunar build <file.ln> [-o out] [--release] [--emit-llvm out.ll]
lunar run   <file.ln> [--release] [-- args...]
lunar test  <file.ln> [filter] [--release]
lunar emit-llvm <file.ln> [-o out.ll] [--release] [--test]
lunar explain <code>     # e.g. borrow::shrink_while_borrowed
lunar explain [--list]
```

`--release` enables LLVM optimizations and turns integer overflow checks into
wrapping arithmetic.

`emit-llvm` prints the module's LLVM IR (after the optimization pipeline, so
pass `--release` to see optimized code) to stdout without building anything.
`build --emit-llvm out.ll` writes the same IR alongside a normal build.

`explain` prints a longer write-up of a diagnostic code, with an erroneous
and a fixed example. It also accepts the code as printed, e.g.
`lunar explain 'err(type::mismatch)'`; without a code it lists every code.
The explanations live in [docs/errors](docs/errors), one file per code.

## Repository layout

| Path | Contents |
|---|---|
| `crates/ln-lexer` | Tokens (logos) |
| `crates/ln-parse-kit` | Minimal parser framework, generic over token kinds |
| `crates/ln-parser` | Lunar grammar → AST |
| `crates/ln-ast` | Syntax tree |
| `crates/ln-diag` | Spans, scoped error codes, ariadne rendering |
| `crates/ln-sema` | Name resolution, type checking, traits, borrow checking → typed IR |
| `crates/ln-codegen` | Monomorphization and LLVM code generation (inkwell) |
| `crates/ln-driver` | The `lunar` CLI, linking, end-to-end tests |
| `std/core`, `std/std` | The standard library, written in Lunar (embedded in the compiler) |
| `docs/errors` | One explanation per diagnostic code, shown by `lunar explain` (embedded in the compiler) |
| `examples/` | Example programs (with expected output in `.out` files) |
| `tests/ui`, `tests/run` | Diagnostics tests and run tests |

Set `LUNAR_STD_DIR=std` to make the compiler read the standard library from
disk instead of the copy embedded at build time.

## Distribution

The `lunar` binary links LLVM dynamically, so it needs LLVM 20 at run time,
just as it does at build time. Release builds are meant to ship the LLVM
shared library next to the binary, the way Rust's toolchain bundles its
LLVM.
