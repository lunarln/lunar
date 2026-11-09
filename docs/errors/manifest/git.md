# manifest::git: a git dependency could not be fetched

`lunar` clones git dependencies with the `git` command and caches them
under `~/.lunar` (or `$LUNAR_HOME`). This code is reported when:

* `git` is not installed or not on `PATH`;
* the repository cannot be reached: a wrong URL, no network, or a private
  repository without credentials;
* the `tag`, `branch` or `rev` does not exist in the repository.

Check that `git` runs, that `git clone <url>` works by hand, and that the
tag, branch or commit is spelled as it is in the repository.

## Erroneous example

The repository has a tag `v1.2.0`, but no tag `1.2.0`:

```toml
[dependencies]
json = { git = "https://github.com/x/json", tag = "1.2.0" }
```

## Fixed

The tag is spelled as it is in the repository:

```toml
[dependencies]
json = { git = "https://github.com/x/json", tag = "v1.2.0" }
```
