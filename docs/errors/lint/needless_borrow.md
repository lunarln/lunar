# lint::needless_borrow: a parameter borrows a small Dull value

A parameter is declared `x: &T`, but `T` is Dull (it has no pointers or
arrays) and at most 16 bytes, so passing it by value makes an independent
copy that is just as cheap as passing its address. Take it by value instead.

Larger Dull values are not reported: borrowing passes an address and avoids
copying the whole value.

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
    println("{double(&21)}");
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
