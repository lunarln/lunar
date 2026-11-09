# name::not_a_value: a value was expected, but the name refers to something else

The path was found, but it does not name a value, so it cannot be used as an
expression. This is reported for:

* an enum variant or struct that has fields, used without them, as in
  `Shape::Circle` where `Circle` holds an `f64`. Only unit variants and
  unit structs are values on their own; tuple variants must be called
  (`Shape::Circle(1.0)`) and named-field variants written with braces
  (`Shape::Rect { w: 1.0, h: 2.0 }`);
* a name or path that resolves to a module, a type or a trait, such as
  `std::io` ("expected a value, found module `std::io`"), `i32`, or a
  named-field struct written without its fields (build it with a struct
  literal, `Point { x: 1, y: 2 }`).

A name that does not exist at all is `name::unresolved`.

## Erroneous example

```lunar
enum Shape {
    Circle(f64),
    Square(f64),
}

fn area(s: Shape): f64 {
    return match s {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Square(a) => a * a,
    };
}

fn main() {
    let s = Shape::Circle;
    println("{area(s)}");
}
```

## Fixed

The variant is constructed with its field:

```lunar
enum Shape {
    Circle(f64),
    Square(f64),
}

fn area(s: Shape): f64 {
    return match s {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Square(a) => a * a,
    };
}

fn main() {
    let s = Shape::Circle(1.0);
    println("{area(s)}");
}
```
