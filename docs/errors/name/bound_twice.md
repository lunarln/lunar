# name::bound_twice: a name is bound twice in one pattern or parameter list

A pattern binds each name once, and so does a function's or closure's
parameter list: in `(x, x)` or `fn f(x: i32, x: i32)` it would be unclear
which value `x` means.

The alternatives of a `|` pattern are the exception: only one of them
matches, so each binds the pattern's variables (`Ok(v) | Err(v)`). One
alternative still cannot bind a name twice, and neither can a `|` pattern
and the pattern around it (`(Some(x) | None, x)`).

The error points at the second binding and at the first.

## Erroneous example

```lunar
fn main() {
    let pair = (1, 2);
    let (x, x) = pair;
    println("{x}");
}
```

## Fixed

Give each binding its own name, or write `_` for the one you don't need:

```lunar
fn main() {
    let pair = (1, 2);
    let (x, y) = pair;
    println("{x} {y}");
}
```
