# type::not_a_place: a value is used where a place is required

Some expressions need a *place*, a location in memory that can be written:
a variable, a field, an index expression or a dereferenced pointer (`*ptr`).
A temporary value such as a literal, the result of a call or an arithmetic
expression is not a place. This error is reported for:

* the left-hand side of an assignment (`=`) or compound assignment (`+=`,
  `-=`, ...) that is not a place;
* an argument for an in-out (`x: &mut T`) parameter that is not a place.

An in-out parameter writes back to the caller's variable, so the argument
has to name something that can receive the result. To pass a computed
value, bind it with `let mut` first, or, if the caller does not need the
result, have the function take its own copy with `mut x: T`.

## Erroneous example

```lunar
fn double(x: &mut i32) {
    x *= 2;
}

fn main() {
    double(21);
}
```

## Fixed

The argument is now a mutable variable, which also receives the result:

```lunar
fn double(x: &mut i32) {
    x *= 2;
}

fn main() {
    let mut n = 21;
    double(n);
    println("{n}");
}
```
