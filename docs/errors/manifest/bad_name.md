# manifest::bad_name: a package or dependency name is not allowed

A package's `name`, and the key of each entry in `[dependencies]`, must
start with a lowercase letter and contain only lowercase letters, digits,
`-` and `_`. A dependency is reachable in code by its key, with `-` turned
into `_` (`my-util` is `use my_util::...;`), so the key must make a valid
path segment.

The dependency keys `core`, `std`, `crater`, `self` and `super` are
reserved: they already mean something at the start of a path. Give such a
dependency another key, and name the package it refers to with
`package = "..."`.

## Erroneous example

```toml
[package]
name = "My App"
version = "0.1.0"

[dependencies]
Json = { path = "../json" }
```

## Fixed

The package name has no capitals or spaces, and the dependency key is
lowercase:

```toml
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
json = { path = "../json" }
```
