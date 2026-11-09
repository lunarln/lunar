# borrow::wrong_marker: the call lends an argument differently from how the parameter takes it

The marker on the argument has to match the parameter:

| Parameter | Write |
|---|---|
| `x: T` (takes ownership) | `f(v)`, no marker |
| `x: &T` (borrows) | `f(&v)` |
| `x: &mut T` (in-out) | `f(&mut v)` |

This error is reported when an argument is lent with `&` to an in-out
parameter (the call would claim the value is only read), with `&mut` to a
borrowing one (the call would claim it may change), or with either to a
parameter that takes ownership (nothing is lent: the value is moved, or
copied if it is Dull).

## Erroneous example

```lunar
fn len(s: &String): usize {
    return s.len();
}

fn main() {
    let mut s = String::from("lunar");
    println("{len(&mut s)}");
}
```

## Fixed

```lunar
fn len(s: &String): usize {
    return s.len();
}

fn main() {
    let s = String::from("lunar");
    println("{len(&s)}");
}
```
