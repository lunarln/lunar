# type::duplicate_field: a struct literal sets the same field twice

A struct literal, or a struct-like enum variant literal, names the same field
more than once. Each field gets exactly one value; it is not clear which of
the two was meant, so this is rejected instead of silently keeping the last
one.

## Erroneous example

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, y: 2, x: 3 };
    println("{p.x}");
}
```

## Fixed

Each field now appears once:

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 3, y: 2 };
    println("{p.x}");
}
```
