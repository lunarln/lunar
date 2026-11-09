# manifest::wrong_package: a dependency points at a different package

The package a dependency points at must have the expected `name` in its
own `lunar.toml`. The expected name is the dependency's key, or the value
of `package = "..."` when the dependency is renamed. This catches a `path`
or `git` that points at the wrong place.

To use a package under a different name in code, keep its real name in
`package` and choose the key freely: `util = { path = "../x", package =
"real-name" }` is used as `use util::...;`.

## Erroneous example

`../helpers/lunar.toml` has `name = "string-helpers"`:

```toml
[dependencies]
helpers = { path = "../helpers" }
```

## Fixed

The dependency is renamed: the key stays `helpers`, and `package` names
the package:

```toml
[dependencies]
helpers = { path = "../helpers", package = "string-helpers" }
```
