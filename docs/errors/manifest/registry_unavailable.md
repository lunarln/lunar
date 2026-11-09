# manifest::registry_unavailable: registry dependencies are not available yet

A dependency given only as a version, `json = "1.2"`, or as a table with
only `version`, `json = { version = "1.2" }`, would come from the package
registry. The registry (planned at craters.dev) is not available yet, so
these dependencies are parsed but cannot be fetched.

For now, depend on the package by `path` or `git`. A `version` can stay
alongside either; it is then checked against the package found.

## Erroneous example

```toml
[dependencies]
json = "1.2"
```

## Fixed

The dependency comes from the package's git repository:

```toml
[dependencies]
json = { git = "https://github.com/x/json", tag = "v1.2.0", version = "1.2" }
```
