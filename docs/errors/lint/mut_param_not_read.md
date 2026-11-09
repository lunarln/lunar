# lint::mut_param_not_read: a `mut` parameter is changed but never read

A parameter declared `mut x: T` is the function's own value: it took
ownership of the argument (or a copy of it, for Dull types), and changing
it does not affect the caller. This parameter is changed but its new value
is never used, which almost always means the caller's variable was meant to
change.

To change the caller's variable, take the parameter in-out with `&mut`:
the caller then passes a mutable place and sees the changes after the call.

## Erroneous example

```lunar
fn bump(mut x: i32) {
    x += 1;
}

fn main() {
    let n = 1;
    bump(n);
    println("{n}");
}
```

## Fixed

The parameter is in-out, so `n` becomes 2:

```lunar
fn bump(x: &mut i32) {
    x += 1;
}

fn main() {
    let mut n = 1;
    bump(n);
    println("{n}");
}
```
