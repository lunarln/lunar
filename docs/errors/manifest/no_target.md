# manifest::no_target: the package has nothing for this command to use

A package can have a program, `src/main.ln`, a library, `src/lib.ln`, or
both. This code is reported when the one a command needs is missing:

* `lunar build` and `lunar run` build the program, so the package must
  have `src/main.ln`;
* a dependency is used through its library, so the depended-on package
  must have `src/lib.ln`.

A package created with `lunar new <name> --lib` has only a library. Its
`#[test]`s still run with `lunar test`. To also make a program, add
`src/main.ln`; to share code from a program package, move that code into
`src/lib.ln`.

## Erroneous example

`app` depends on `tool`, a package with only a program:

```text
tool/lunar.toml
tool/src/main.ln
```

## Fixed

The code `app` uses is moved into a library, `tool/src/lib.ln`:

```text
tool/lunar.toml
tool/src/lib.ln
tool/src/main.ln
```
