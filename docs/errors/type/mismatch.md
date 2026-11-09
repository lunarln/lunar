# type::mismatch: a value has a different type than the one expected

An expression was used where a value of another type was required: the
initializer of an annotated `let`, a function argument, a return value, an
assigned value, a struct field, the branches of an `if` or `match`, and so on.

Lunar has no implicit conversions between unrelated types. The only implicit
conversions are the ones the language defines for arguments, such as passing
an owning array to a slice parameter or a string literal to a `FmtArgs`
parameter. Everywhere else the types must match exactly; for example a
`[i32; ..+]` does not silently become a `[i32]` in a `let`.

The same code is also used for a few related errors:

* `return;` without a value in a function that declares a return type;
* `?` used on an `Option` in a function that returns a `Result` (or the other
  way around, or with different error types);
* passing a slice view as the receiver of a `&mut self` method.

## Erroneous example

```lunar
fn half(n: i32): i32 {
    return n / 2;
}

fn main() {
    let x: i64 = half(10);
    println("{x}");
}
```

## Fixed

Make the types agree, here by declaring `x` with the type that `half`
returns (an explicit `as i64` cast would also work):

```lunar
fn half(n: i32): i32 {
    return n / 2;
}

fn main() {
    let x: i32 = half(10);
    println("{x}");
}
```
