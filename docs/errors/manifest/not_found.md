# manifest::not_found: a directory has no `lunar.toml`

A package is a directory with a `lunar.toml`. This code is reported when:

* a `path` dependency points at a directory without one (paths are
  relative to the directory of the `lunar.toml` that names them);
* a workspace member listed in `members` has none;
* a command that works on a project (`lunar build`, `lunar run`,
  `lunar test`, `lunar add`, ...) is run without a file argument, and
  neither the current directory nor any of its parents has a `lunar.toml`.

Check the path for typos. To build a single file outside a project, pass
it: `lunar run hello.ln`. To start a project, run `lunar new <name>` or
`lunar init`.

The examples below use this layout, where `util` is a sibling of `app`,
not a directory inside it:

```text
projects/
  app/lunar.toml
  app/src/main.ln
  util/lunar.toml
  util/src/lib.ln
```

## Erroneous example

```toml
# projects/app/lunar.toml
[package]
name = "app"
version = "0.1.0"

[dependencies]
util = { path = "util" }
```

## Fixed

The path is relative to `app/`, so it goes up one directory first:

```toml
# projects/app/lunar.toml
[package]
name = "app"
version = "0.1.0"

[dependencies]
util = { path = "../util" }
```
