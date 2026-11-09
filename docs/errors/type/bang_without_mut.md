# type::bang_without_mut: a `!` parameter is not declared `mut`

A parameter or receiver has a `!T` type but is not `mut`. `!` says the
function may shrink the argument, and shrinking is a mutation. Without `mut`
the parameter is read-only, so the function could never actually shrink it,
yet every caller would still have to give up its views into the argument.

Write `mut x: !T` for parameters and `mut self: !Self` for receivers. If the
function does not shrink the value, drop the `!` instead.

## Erroneous example

```lunar
fn drop_last(s: &![i32; -..+]) {
    s.pop();
}

fn main() {
    let mut a: [i32; -..+] = [1, 2];
    drop_last(a);
    println("{a.len()}");
}
```

## Fixed

Declare the parameter `mut`:

```lunar
fn drop_last(s: &mut ![i32; -..+]) {
    s.pop();
}

fn main() {
    let mut a: [i32; -..+] = [1, 2];
    drop_last(a);
    println("{a.len()}");
}
```
