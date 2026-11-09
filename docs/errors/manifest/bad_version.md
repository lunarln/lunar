# manifest::bad_version: a version or version requirement is malformed

A package's `version` must be a semantic version: three numbers
`major.minor.patch`, optionally followed by a pre-release (`-beta.1`) or
build metadata (`+abc`). `"1.0"` and `"v1.0.0"` are not versions.

A dependency's `version` is a requirement, not a version, and follows
Cargo's rules: `"1.2"` means `^1.2`, that is >=1.2.0 and <2.0.0; `"0.3"`
means >=0.3.0 and <0.4.0; `"=1.2.4"` means exactly 1.2.4. This code is
reported when the requirement cannot be parsed.

## Erroneous example

```toml
[package]
name = "app"
version = "0.1"

[dependencies]
json = { git = "https://github.com/x/json", version = "about 1.2" }
```

## Fixed

The package version has all three numbers, and the requirement is a
version requirement:

```toml
[package]
name = "app"
version = "0.1.0"

[dependencies]
json = { git = "https://github.com/x/json", version = "1.2" }
```
