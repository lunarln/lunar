# type::bang_without_mut: a `!` parameter is not `&mut`

A parameter or receiver has a `!T` type but is not in-out (`&mut`). `!` says
the function may shrink the caller's value in place, so the caller must see
the result: that is what `&mut` means. A borrowed `x: &!T` is read-only, so
the function could never shrink it, and an owned `x: !T` is the function's
own value, which it may shrink anyway, yet every caller would still have to
give up its views into the argument.

Write `x: &mut !T` for parameters and `&mut !self` for receivers. If the
function does not shrink the caller's value, drop the `!` instead.

## Erroneous example

```lunar
fn drop_last(s: &![i32; -..+]) {
    s.pop();
}

fn main() {
    let mut a: [i32; -..+] = [1, 2];
    drop_last(&a);
    println("{a.len()}");
}
```

## Fixed

Declare the parameter `&mut !`:

```lunar
fn drop_last(s: &mut ![i32; -..+]) {
    s.pop();
}

fn main() {
    let mut a: [i32; -..+] = [1, 2];
    drop_last(&mut a);
    println("{a.len()}");
}
```
