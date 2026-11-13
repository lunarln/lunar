# Lunar

[![CI](https://github.com/lunarln/lunar/actions/workflows/ci.yml/badge.svg)](https://github.com/lunarln/lunar/actions/workflows/ci.yml)

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
* **Parameter modes**: `x: T` takes ownership, `x: &T` borrows, `x: &mut T`
  is in-out. Call sites look the same either way, and the compiler catches
  any use of a moved value.
* **Growing and shrinking**: a `&mut` parameter may shrink or replace its
  argument, so lending a value in-out ends the views into it. A
  `&mut +T` parameter promises to only grow it (`push`, `push_str`), so
  views survive. A function that stores a view it was given into an
  in-out argument marks that parameter `kept`. The compiler checks all of
  this.
* **`Dull` and `Light`**: compiler-implemented traits. Dull types hold no
  pointers or arrays; Light types own no array or string (Dull values and
  views such as `str`). Only Light types can be array items, so
  `[str; ..+]` works and `[String; ..+]` is `err(type::array_owner_items)`.
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

## Installing

Prebuilt compilers for Linux (x86_64), macOS (Apple silicon) and Windows
(x64) are on the [releases page](https://github.com/lunarln/lunar/releases).
The install scripts download the latest one for your platform, check its
SHA-256 checksum and unpack it into `~/.lunar`:

```sh
# Linux, macOS
curl -fsSL https://raw.githubusercontent.com/lunarln/lunar/main/install.sh | sh
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/lunarln/lunar/main/install.ps1 | iex
```

Then add `~/.lunar/bin` to your `PATH` (the scripts print how). Pick
another directory with `--prefix DIR` (`-Prefix DIR`) or `LUNAR_PREFIX`,
and a version with `--version 0.1.0` (`-Version 0.1.0`) or
`LUNAR_VERSION`.

A release archive (`lunar-<version>-<target>.tar.gz`, or `.zip` on Windows)
can also be unpacked anywhere by hand. It holds:

```
bin/lunar(.exe)              the compiler, package manager and language server
bin/LLVM-C.dll               (Windows) LLVM, found next to the executable
lib/libLLVM.*                (Linux, macOS) LLVM and the libraries it needs,
                             found through the executable's run path
lib/lunar/std/               the standard library
lib/lunar/docs/errors/       the explanations shown by `lunar explain`
LICENSE-*, licenses/         Lunar's licenses, LLVM's and the bundled libraries'
```

LLVM comes with the compiler, so nothing else is needed to run it. To
build programs it needs a linker, which you install yourself:

* **Linux**: a C toolchain that provides `cc` (gcc or clang; on Debian and
  Ubuntu, `build-essential`). The prebuilt compiler needs glibc 2.34 and
  the C++ runtime of GCC 12 or newer (Ubuntu 22.04, Debian 12, Fedora 36
  and later).
* **macOS**: the Xcode command line tools (`xcode-select --install`). The
  prebuilt compiler needs macOS 15 or newer on Apple silicon.
* **Windows**: the
  [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  with the C++ workload (for the C runtime and the Windows SDK), and either
  LLVM `clang` on `PATH` (it finds the Build Tools by itself) or a
  Developer PowerShell for Visual Studio, which puts `link.exe` on `PATH`.

`LUNAR_CC` overrides the linker everywhere. Without one, `lunar build`
fails with `err(build::link)` and says what to install.

## Building

The compiler is written in Lunar ([craters/](craters/README.md)) and is
built by a released Lunar compiler, the one named in
[craters/stage0.txt](craters/stage0.txt). Requirements:

* that release, installed as above (with `--version`), or unpacked anywhere;
* LLVM 20, whose shared library the compiler links. Point `LUNAR_LIB_PATH`
  at its directory:
  * Windows: the official installer from
    [llvm-project releases](https://github.com/llvm/llvm-project/releases);
    `LUNAR_LIB_PATH=C:/Program Files/LLVM/lib`, and `LLVM-C.dll` (in
    `C:\Program Files\LLVM\bin`) on `PATH` to run what you build;
  * Linux: `llvm-20-dev` (e.g. from [apt.llvm.org](https://apt.llvm.org));
    `LUNAR_LIB_PATH=$(llvm-config-20 --libdir)`;
  * macOS: `brew install llvm@20`;
    `LUNAR_LIB_PATH=$(brew --prefix llvm@20)/lib`;
* a linker, as above.

```sh
cd craters
lunar build --release      # craters/target/release/lunar
target/release/lunar run ../examples/arrays.ln
```

The compiler built in `craters/target/` reads the standard library and the
error explanations of the checkout it is in. `tools/bootstrap.sh STAGE0 DIR`
does what CI does: stage 1 is `craters/` built by `STAGE0` (with the std of
its own release), stage 2 is built by stage 1 and stage 3 by stage 2 (with
the repository's std), and stages 2 and 3 must be identical.

The tests run against a built compiler:

```sh
cd craters
lunar test -p lunar                     # the compiler's own tests
LUNAR_BIN=$PWD/target/release/lunar lunar test -p lunar-tests   # the executable
```

[CONTRIBUTING.md](CONTRIBUTING.md) has the bootstrap rule (what the
compiler's own code may use) and how to move stage 0 to a new release.

## Usage

`lunar` is both the compiler and the project manager. In a project (a
directory with a `lunar.toml`), commands work on the whole package:

```text
lunar new hello [--lib]  # create a package: lunar.toml, src/main.ln (or src/lib.ln)
lunar init [--lib]       # the same, in the current directory
lunar build [--release]  # build src/main.ln to target/<profile>/hello
lunar run [-- args...]   # build and run it
lunar test [filter]      # run #[test]s in src/lib.ln, src/main.ln and tests/*.ln
lunar check              # type-check without generating code
lunar add util --path ../util
lunar add json --git https://github.com/x/json --tag v1.2.0
lunar remove json
lunar update             # move git dependencies to their newest commit; rewrite lunar.lock
lunar clean              # remove target/
```

```toml
# lunar.toml
[package]
name = "hello"
version = "0.1.0"

[dependencies]
util = { path = "../util" }
json = { git = "https://github.com/x/json", tag = "v1.2.0", version = "1.2" }
```

A dependency's library is reachable by its name, like `std`:
`use json::Value;`. Only declared dependencies are visible, not their own
dependencies. `version` requirements use Cargo's `^` rules, and `lunar.lock`
records the exact commit of every git dependency. Registry dependencies
(`json = "1.2"`) are parsed, but the registry is not available yet. Git
checkouts are cached in `~/.lunar` (or `$LUNAR_HOME`).

A workspace groups packages that are built together. Its members share one
`lunar.lock` and one `target/` at the workspace root, and depend on each
other by `path`:

```toml
# lunar.toml at the workspace root (it may also have a [package])
[workspace]
members = ["crates/*", "tools/gen"]
```

Inside a member, commands act on that member; at the root they act on every
member. `-p <name>` picks one member from anywhere in the workspace:
`lunar run -p app`, `lunar test -p parser`.

Manifest problems are reported like compiler errors, pointing into
`lunar.toml` with a `manifest::...` code that `lunar explain` describes.

Given a file, commands work on that file alone:

```text
lunar check <file.ln> [--target-os linux|macos|windows]
lunar build <file.ln> [-o out] [--release] [--emit-llvm out.ll]
lunar run   <file.ln> [--release] [-- args...]
lunar test  <file.ln> [filter] [--release]
lunar emit-llvm <file.ln> [-o out.ll] [--release] [--test]
lunar explain <code>     # e.g. borrow::shrink_while_borrowed
lunar explain [--list]
lunar lsp                # language server over stdio, for editors
```

`--release` enables LLVM optimizations and turns integer overflow checks into
wrapping arithmetic.

`emit-llvm` prints the module's LLVM IR (after the optimization pipeline, so
pass `--release` to see optimized code) to stdout without building anything.
`build --emit-llvm out.ll` writes the same IR alongside a normal build.

### Debugging

The compiler generates debug information the way cargo does: full in the debug profile, none with
`--release`. `--debug-info none|lines|full` overrides that for `build`,
`run`, `test` and `emit-llvm` (`lines` is functions and line tables only,
enough for backtraces and stepping).

* Windows: CodeView, in `<program>.pdb` next to the executable, with the
  visualizers of [std/debugger/lunar.natvis](std/debugger/lunar.natvis)
  embedded for Visual Studio and WinDbg. `LUNAR_DEBUG_FORMAT=dwarf` puts
  DWARF in the executable instead (linked with `lld`), for lldb, which reads
  no PDBs as LLVM's Windows installer builds it.
* Linux: DWARF 5 in the executable. macOS: DWARF 4, collected by
  `dsymutil` into `<program>.dSYM`.

Functions are named by their path (`demo::Point::sum`, `demo::max::<i32>`,
`demo::main::{closure#0}`), and parameters and variables are visible with
their types: structs with their fields, `str` and `String` as `data_ptr` and
`length`, arrays and slices as `data_ptr`, `length` (and `capacity`), and
enums as a `tag` naming the variant beside a member per variant. With lldb,
[std/debugger/lunar_lldb.py](std/debugger/lunar_lldb.py) shows text,
elements and only the active variant:

```text
(lldb) command script import <lunar>/std/debugger/lunar_lldb.py
(lldb) frame variable
(String) label = "hello world"
(demo::Shape) shape = Rect { w = 2, h = 5 }
(core::option::Option::<i32>) maybe = Some(7)
```

Source paths under the directory the build runs in are recorded relative to
it; `--remap-path-prefix FROM=TO` rewrites the directories, so builds in
different places can record the same paths.

`explain` prints a longer write-up of a diagnostic code, with an erroneous
and a fixed example. It also accepts the code as printed, e.g.
`lunar explain 'err(type::mismatch)'`; without a code it lists every code.
The explanations live in [docs/errors](docs/errors), one file per code.

## Editor support

`lunar lsp` is a language server: diagnostics as you type, quick fixes,
hover, go to definition (also into the standard library), references,
rename, completion, signature help, semantic tokens, symbols and inlay
hints. The VS Code extension in [editors/vscode](editors/vscode) adds
syntax highlighting and runs the `lunar` on your `PATH` (or the one the
install scripts put in `~/.lunar/bin`):

```sh
cd editors/vscode && npm install && npx @vscode/vsce package
code --install-extension lunar-0.1.0.vsix
```

## Repository layout

| Path | Contents |
|---|---|
| `craters/` | The compiler, written in Lunar and self-hosted: lexer, parser, semantic analysis, LLVM code generation, the `lunar` command, package manager and language server (see [craters/README.md](craters/README.md)) |
| `craters/lunar-tests` | The tests of the `lunar` executable, run against a built compiler |
| `craters/stage0.txt` | The release that builds the compiler |
| `std/core`, `std/std` | The standard library, written in Lunar (installed next to the compiler) |
| `std/debugger` | Debugger visualizers (natvis, lldb) |
| `docs/errors` | One explanation per diagnostic code, shown by `lunar explain` |
| `examples/` | Example programs (with expected output in `.out` files) |
| `tests/ui`, `tests/run`, `tests/fatal` | Diagnostics tests, run tests, tests of clean runtime failures |
| `tests/lsp` | Language server sessions |
| `tools/` | Bootstrapping and packaging, for CI and releases |
| `editors/vscode` | VS Code extension: syntax highlighting, LSP client |
| `install.sh`, `install.ps1` | The install scripts |

`LUNAR_STD_DIR` makes the compiler read the standard library from another
directory than its own (`<prefix>/lib/lunar/std`, or the checkout's `std/`).

## Releases

`lunar --version` prints the version in `craters/lunar/lunar.toml`
(through `env!("LUNAR_PKG_VERSION")`). Pushing a tag `v<that version>` runs
[`.github/workflows/release.yml`](.github/workflows/release.yml), which on
each platform:

1. bootstraps the compiler (`tools/bootstrap.sh`): stage 1 is `craters/`
   built by the stage 0 compiler, stage 2 is built by stage 1, stage 3 by
   stage 2, and stages 2 and 3 must be identical;
2. packages stage 2 with the standard library, the error explanations and
   LLVM's shared library (`tools/package.sh`), rewriting the executable's
   run path on Linux (`$ORIGIN/../lib`) and macOS
   (`@executable_path/../lib`) so that it loads the bundled LLVM, and tries
   the package out;
3. unpacks each archive on a clean machine and builds programs with it;
4. publishes the archives and their `.sha256` checksums as a GitHub
   release.

Run by hand, or on a pull request that changes the release tooling, it
does everything but publish.
