# manifest::lock: `lunar.lock` is corrupted or invalid

`lunar.lock` records the exact commit of every git dependency and the
source of every package that is not a workspace member, so that builds
are repeatable. `lunar` writes it; it is not meant to be edited by hand.
This code is reported when the file is not valid TOML, has entries in the
wrong shape, or was edited into a state `lunar` cannot use.

The lock file holds nothing that cannot be rebuilt from `lunar.toml`.
Delete it and let `lunar update` write a new one. Git dependencies then
move to the newest commit their `tag`, `branch` or `rev` allows.

## Erroneous example

A merge left conflict markers in `lunar.lock`:

```text
<<<<<<< HEAD
...
=======
...
>>>>>>> feature
```

## Fixed

The lock file is regenerated:

```text
rm lunar.lock
lunar update
```
