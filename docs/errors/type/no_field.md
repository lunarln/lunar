# type::no_field: a field that does not exist is accessed or named

A field access `x.name` or `x.0` refers to a field the type does not have.
The diagnostic lists the fields that do exist. Arrays, `str` and `String`
have no fields at all; their length is the method `.len()`.

The same code is reported for:

* a struct literal or struct pattern naming a field the struct or variant
  does not declare, such as `Point { x: 1, z: 2 }`;
* a tuple index that is out of range, like `.2` on a pair, or a tuple index
  on a type that has no tuple fields.

## Erroneous example

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, y: 2 };
    println("{p.z}");
}
```

## Fixed

Use a field that the struct declares:

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, y: 2 };
    println("{p.y}");
}
```
