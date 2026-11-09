# manifest::parse: `lunar.toml` could not be read as a manifest

A `lunar.toml` must be valid TOML and contain only the keys Lunar knows.
This code is reported when:

* the file is not valid TOML (an unclosed string, a missing `=`, a
  duplicated key, ...);
* a key is unknown, such as a misspelled `[dependancies]` or `verison`;
* a value has the wrong type, such as `version = 1` instead of a string;
* the file has neither a `[package]` nor a `[workspace]` section, or the
  command needs a package and the file only has `[workspace]`.

## Erroneous example

```toml
[package]
name = "app"
verison = "0.1.0"
```

## Fixed

The key is spelled `version`:

```toml
[package]
name = "app"
version = "0.1.0"
```
