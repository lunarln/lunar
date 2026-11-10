# type::grow_without_mut: a `+` parameter is not `&mut`

A parameter or receiver has a `+T` type but is not in-out (`&mut`). `+`
promises that the function only grows the caller's value, which is only
something to promise about a value the function changes in place. A
borrowed `x: &+T` cannot change the caller's value at all, and an owned
`x: +T` is the function's own value.

Write `x: &mut +T` for parameters and `&mut +self` for receivers, or drop
the `+`.

## Erroneous example

```lunar
fn count(log: &+[i32; ..+]): usize {
    return log.len();
}

fn main() {
    let log: [i32; ..+] = [1, 2];
    println("{count(&log)}");
}
```

## Fixed

The function only reads the log, so drop the `+`:

```lunar
fn count(log: &[i32; ..+]): usize {
    return log.len();
}

fn main() {
    let log: [i32; ..+] = [1, 2];
    println("{count(&log)}");
}
```
