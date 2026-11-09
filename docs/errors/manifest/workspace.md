# manifest::workspace: the workspace is inconsistent, or the command is ambiguous in it

A `lunar.toml` with a `[workspace]` section groups several packages, its
`members`. A member is a directory relative to the workspace root; a
trailing `/*` means every subdirectory that has a `lunar.toml`. Members
share one `lunar.lock` and one `target/` at the root, and depend on each
other by `path`. The root may also have a `[package]` of its own. This
code is reported when:

* two members have the same package `name`; names must be unique in a
  workspace;
* `-p <name>` names no member of the workspace;
* `lunar run` is run at the workspace root and several members have a
  program; pick one with `-p <name>`.

A package inside the workspace directory but not listed in `members` is
not part of the workspace: it is built as a standalone package, with its
own `lunar.lock` and `target/`.

## Erroneous example

`crates/*` matches `crates/parser`, and both it and `parser` have
`name = "parser"`:

```toml
[workspace]
members = ["parser", "crates/*"]
```

## Fixed

The old copy is no longer a member (or one of the two packages is
renamed):

```toml
[workspace]
members = ["crates/*"]
```
