# build::no_main: the program has no `main` function

`lunar build`, `lunar run` and `lunar emit-llvm` produce an executable,
which starts by calling the function `main` in the root file. The program
being built does not define one.

`lunar check` does not require `main`, so a library-like file can be
checked on its own. `lunar test` does not need it either: it builds a test
harness that runs the `#[test]` functions instead.

`main` must take no parameters and return `()` or `i32`; a `main` with a
different signature is `err(type::bad_main)`.

The current compiler prints this as a plain ``error: `file.ln` has no
`main` function`` line rather than with this code, and only when building,
so the examples below are not checked automatically.

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
