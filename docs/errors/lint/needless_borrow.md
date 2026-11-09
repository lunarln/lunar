# lint::needless_borrow: a parameter borrows a Dull value

A parameter is declared `x: &T`, but `T` is Dull: it has no pointers or
arrays, so passing it by value already makes a cheap, independent copy.
Borrowing it gains nothing, so take it by value instead.

`&mut T` on a Dull type is different and is not reported: it makes the
parameter in-out, so the function can change the caller's variable.

Some borrows are never reported:

* `&self`: methods borrow their receiver by convention, whatever its type,
  so all of a type's methods read alike and the type can later gain an owned
  field without every signature changing.
* Trait methods and their implementations, because an impl must match the
  trait's declaration even when its own types are Dull.

## Erroneous example

```lunar
fn double(x: &i32): i32 {
    return x * 2;
}

fn main() {
    println("{double(21)}");
}
```

## Fixed

The parameter takes its value directly:

```lunar
fn double(x: i32): i32 {
    return x * 2;
}

fn main() {
    println("{double(21)}");
}
```
