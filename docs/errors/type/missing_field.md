# type::missing_field: a struct literal leaves out a field

A struct literal, or a struct-like enum variant literal such as
`Shape::Rect { w: 1.0, h: 2.0 }`, must give a value for every field. Lunar
has no default field values and no uninitialized memory, so a value with a
field left out cannot be built.

The error lists every missing field. Fields can be written in any order, and
the shorthand `Point { x, y }` takes the values of local variables named
`x` and `y`.

## Erroneous example

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1 };
    println("{p.x}");
}
```

## Fixed

The literal now sets `y` as well:

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, y: 0 };
    println("{p.x} {p.y}");
}
```
