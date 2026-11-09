# type::arg_count: a call has the wrong number of arguments

A function, method or function pointer was called with more or fewer
arguments than it declares parameters. Lunar has no default arguments and no
overloading, so the counts must match exactly. For methods, the receiver
before the `.` counts as the `self` parameter and is not written again
inside the parentheses.

The same code is reported when a tuple-like enum variant or struct
constructor such as `Some(x)` is given the wrong number of fields.

## Erroneous example

```lunar
fn area(w: i32, h: i32): i32 {
    return w * h;
}

fn main() {
    println("{area(4)}");
}
```

## Fixed

Pass one argument for every parameter:

```lunar
fn area(w: i32, h: i32): i32 {
    return w * h;
}

fn main() {
    println("{area(4, 5)}");
}
```
