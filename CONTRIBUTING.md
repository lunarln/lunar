# Contributing to Lunar

How to build the compiler and run the tests is in the
[README](README.md#building). This file is about the bootstrap: which
compiler builds which code, what that allows, and how to move it forward.

## The bootstrap

The compiler is written in Lunar (`craters/`). It is built by a released
compiler, **stage 0**, the version in [craters/stage0.txt](craters/stage0.txt):

| Stage | Compiler | Built from | With the standard library |
|---|---|---|---|
| 0 | the release in `craters/stage0.txt` | (downloaded) | its own (`<prefix>/lib/lunar/std`) |
| 1 | stage 0 | `craters/` | stage 0's own |
| 2 | stage 1 | `craters/` | the repository's `std/` |
| 3 | stage 2 | `craters/` | the repository's `std/` |

Stages 1 and 2 are the same program built by different compilers, so
stages 2 and 3 must be identical, byte for byte. CI checks this
(`tools/bootstrap.sh`) on Linux, macOS and Windows, then runs every test
against stage 2, and releases ship stage 2.

Stage 0 builds stage 1 with **the standard library it was released with**,
not the repository's `std/`, the way rustc's beta compiler builds with its
own std. That way `std/` can start using a language feature as soon as the
compiler in the repository has it.

## The bootstrap rule

* **`craters/`** (the compiler and what it depends on) may use only the
  language features and the standard library APIs of the stage 0 release.
  Stage 0 compiles it, with its own std.
* **`std/`** may use whatever the compiler in the repository supports.
  Stage 0 never compiles it. Stages 1 and later do, and they are built
  from the repository.
* The tests (`craters/lunar-tests`, the `#[test]`s in `craters/`, `tests/`,
  `examples/`) are built by stage 2, so they may use anything in the
  repository.

So a new language feature or std API lands in two steps:

1. Implement it in `craters/` and use it in `std/`, tests and examples as
   you like, but not in `craters/` itself.
2. After a release that has it, bump stage 0 to that release. Only then
   may `craters/` use it.

If `craters/` uses something too new, CI fails while stage 0 builds
stage 1. To check locally, run `tools/bootstrap.sh` with the stage 0
compiler (`sh install.sh --prefix /tmp/stage0 --version $(grep -v '^#'
craters/stage0.txt)`).

## Bumping stage 0

1. Release a version that has what `craters/` needs. Set `version` in
   `craters/lunar/lunar.toml` (that is what `lunar --version` prints), merge
   it to `main`, and push the tag `v<version>`. The release workflow
   (`.github/workflows/release.yml`) builds, checks, packages and publishes
   it. The version in `lunar.toml` must match the tag.
2. Once the release is published, put its version in `craters/stage0.txt`
   (the last line that is not a comment) in a pull request. CI then
   bootstraps from it. The release workflow uses the same file, so the
   next release is built from this one.
3. Now `craters/` may use what the new stage 0 has.

Stage 0 does not need to be the latest release; move it when `craters/`
needs something new. CI caches the stage 0 installation per version.

## Tests

* `lunar test -p lunar` (in `craters/`): the compiler's own tests,
  including the language server sessions in `tests/lsp/*/session.json`
  replayed in process, and the debug information tests (which run lldb or
  `llvm-pdbutil` when they are installed).
* `LUNAR_BIN=<compiler> lunar test -p lunar-tests`: the executable, through
  its command line and language server: `tests/ui`, `tests/run`,
  `tests/fatal`, `examples`, `lunar explain` and `docs/errors`, projects
  and workspaces, the same `tests/lsp` sessions over `lunar lsp`, and the
  language server on broken programs (`LUNAR_ROBUST_CUTS=400` for a
  thorough run).
* The other packages in `craters/` have unit tests too: `lunar test -p
  ln-json`, ...

To add a language server test, add a directory to `tests/lsp` with its
sources and a `session.json`. The format is described at the top of
`craters/lunar/src/ide_tests.ln`.
