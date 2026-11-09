# type::bad_main: `main` has the wrong signature

The `main` function of a program is the entry point, called by the runtime.
It must take no parameters, have no generic parameters, and return either
`()` or `i32`. An `i32` result becomes the process exit status.

Command line arguments are not passed to `main`; read them with
`std::env::arg_count()` and `std::env::arg(i)`.

## Erroneous example

```lunar
fn main(): bool {
    return true;
}
```

## Fixed

`main` now returns an `i32` exit status:

```lunar
fn main(): i32 {
    return 0;
}
```
