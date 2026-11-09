# manifest::bad_dependency: a dependency entry is contradictory or incomplete

Each entry in `[dependencies]` says where the package comes from, and it
must say so exactly once. This code is reported when an entry:

* has both `path` and `git`;
* has more than one of `tag`, `branch` and `rev`, which each pick a
  commit of a git dependency;
* has `tag`, `branch` or `rev` on a `path` dependency, which has no
  commits to pick;
* is a table with none of `version`, `path` and `git`.

A git dependency with none of `tag`, `branch` and `rev` uses the
repository's default branch.

## Erroneous example

```toml
[dependencies]
util = { path = "../util", branch = "main" }
json = { git = "https://github.com/x/json", tag = "v1.2.0", rev = "4f2a9c1" }
```

## Fixed

The path dependency has no git reference, and the git dependency picks its
commit one way:

```toml
[dependencies]
util = { path = "../util" }
json = { git = "https://github.com/x/json", tag = "v1.2.0" }
```
