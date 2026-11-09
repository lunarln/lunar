# type::return_missing: a function can finish without returning a value

A function with a return type other than `()` must produce a value on every
path: through `return value;`, or a tail expression (the final expression of
the body without a `;`). This error is reported when the body has no tail
expression and control can reach its end, for example when a `return` only
happens inside an `if`.

The same code is reported for a function declared without a body
(`fn f(): i32;`) outside a trait or `extern` block. Only trait methods and
extern functions can be declared without a body.

When the body does end in a block-like expression without a value, such as
an `if` without `else` or a `for` loop, that expression is the tail and the
error is reported as `type::mismatch` (expected the return type, found
`()`) instead.

## Erroneous example

```lunar
fn sign(x: i32): i32 {
    if x < 0 {
        return -1;
    }
    let positive = x > 0;
}

fn main() {
    println("{sign(-5)}");
}
```

## Fixed

Every path now returns a value; the last one is a tail expression:

```lunar
fn sign(x: i32): i32 {
    if x < 0 {
        return -1;
    }
    let positive = x > 0;
    if positive { 1 } else { 0 }
}

fn main() {
    println("{sign(-5)}");
}
```
