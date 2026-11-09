# syntax::missing_body: a function has no body

A function was declared with a signature followed by `;` instead of a body
`{ ... }`. Only two kinds of functions are declared without a body:

* trait methods without a default implementation, which every `impl` of the
  trait provides;
* functions in an `extern "C" { ... }` block, which are defined in C.

Everywhere else a function needs a body, even an empty one.

## Erroneous example

```lunar
fn answer(): i32;

fn main() {
    println("{answer()}");
}
```

## Fixed

The function now has a body:

```lunar
fn answer(): i32 {
    return 42;
}

fn main() {
    println("{answer()}");
}
```
