# type::bang_outside_param: a `!T` type is used somewhere other than a parameter

`!T` marks a function parameter whose argument the function may shrink. It
only has meaning on an in-out parameter (`x: &mut !T`, or a `&mut !self`
receiver), where it tells the caller that views into the argument must not
be used after the call. A `let` binding, a struct field, a return type, or a type nested inside
another type such as `[![i32; -..+]; 2]` cannot be `!`.

A local variable that owns a `[T; -..+]` or `String` can already be shrunk
directly, so it needs no `!`.

## Erroneous example

```lunar
fn main() {
    let mut stack: ![i32; -..+] = [];
    stack.push(1);
    stack.pop();
}
```

## Fixed

Drop the `!`; the owner of a shrinkable array can always shrink it:

```lunar
fn main() {
    let mut stack: [i32; -..+] = [];
    stack.push(1);
    stack.pop();
}
```
