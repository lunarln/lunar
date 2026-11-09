# manifest::version_mismatch: a dependency's version does not meet the requirement

When a dependency has a `version` requirement, the `version` in the
depended-on package's `lunar.toml` must satisfy it. For a git dependency,
that is the package at the chosen tag, branch or commit.

Requirements follow Cargo's `^` rules: `"1.2"` accepts >=1.2.0 and
<2.0.0, `"0.3"` accepts >=0.3.0 and <0.4.0 (for 0.x versions the minor
number is the breaking one), and `"=1.2.4"` accepts only 1.2.4.

Either point the dependency at a release that satisfies the requirement,
or, if the code works with the version found, change the requirement.

## Erroneous example

The tag `v2.0.0` holds a package with `version = "2.0.0"`:

```toml
[dependencies]
json = { git = "https://github.com/x/json", tag = "v2.0.0", version = "1.2" }
```

## Fixed

The tag picks a 1.x release:

```toml
[dependencies]
json = { git = "https://github.com/x/json", tag = "v1.2.0", version = "1.2" }
```
