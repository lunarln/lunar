# type::immutable: a value that is not mutable is modified

`let` bindings and plain parameters are immutable. Modifying one, or
anything reached through one, is an error. This error is reported when the
program:

* assigns to it, with `=` or a compound assignment such as `+=`;
* passes it as the argument of a `mut` (in-out) parameter;
* calls a method that takes `mut self` on it, such as `push`;
* takes a `[T; &mut]` slice of it, explicitly or by passing it where a
  mutable slice is expected.

The same applies to elements of a read-only `[T; &]` slice, and to writes
through a `*T` pointer, which needs to be a `*mut T`.

Declare the variable with `let mut`, or the parameter as `mut name: T`
(which makes it in-out, so the caller sees the change).

## Erroneous example

```lunar
fn main() {
    let count = 0;
    count += 1;
    println("{count}");
}
```

## Fixed

The variable is now declared `mut`:

```lunar
fn main() {
    let mut count = 0;
    count += 1;
    println("{count}");
}
```
