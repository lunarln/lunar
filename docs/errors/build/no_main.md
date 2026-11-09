# build::no_main: the program has no `main` function

`lunar build`, `lunar run` and `lunar emit-llvm` produce an executable,
which starts by calling the function `main` in the root file. The program
being built does not define one.

`lunar check` does not require `main`, so a library-like file can be
checked on its own. `lunar test` does not need it either: it builds a test
harness that runs the `#[test]` functions instead.

`main` must take no parameters and return `()` or `i32`; a `main` with a
different signature is `err(type::bad_main)`.

Because `lunar check` does not report it, the erroneous example below is
not checked automatically. Building it reports:

```text
err(build::no_main): `greet.ln` has no `main` function
  help: add `fn main() { ... }` to the file; to run its `#[test]` functions instead, use `lunar test`
```

## Erroneous example

```lunar,ignore
fn greet() {
    println("hello");
}
```

## Fixed

Add a `main` function that the program starts from:

```lunar
fn greet() {
    println("hello");
}

fn main() {
    greet();
}
```
