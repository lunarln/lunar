# type::bad_pattern: a pattern does not fit the shape of the value

A pattern was written that cannot match the type being matched. This error
is reported when:

* a variant or struct with fields is matched by its bare name, as in
  `Shape::Circle` instead of `Shape::Circle(r)` or `Shape::Circle(..)`;
* a tuple or tuple-variant pattern has the wrong number of fields, or more
  fields than the value has when `..` is used;
* a tuple pattern is used on a type that is not a tuple;
* a parenthesized pattern `Name(...)` names something that is not a tuple
  variant or tuple struct;
* a struct pattern leaves out fields without ending in `..`;
* a path pattern names something other than a unit variant, unit struct or
  constant;
* a range pattern (`'a'..='z'`) is used on a type other than an integer or
  `char`.

## Erroneous example

```lunar
enum Shape {
    Circle(f64),
    Square(f64),
}

fn side(s: Shape): f64 {
    return match s {
        Shape::Circle => 0.0,
        Shape::Square(w) => w,
    };
}

fn main() {
    println("{side(Shape::Square(2.0))}");
}
```

## Fixed

The `Circle` arm now matches the variant's field, ignoring it with `..`:

```lunar
enum Shape {
    Circle(f64),
    Square(f64),
}

fn side(s: Shape): f64 {
    return match s {
        Shape::Circle(..) => 0.0,
        Shape::Square(w) => w,
    };
}

fn main() {
    println("{side(Shape::Square(2.0))}");
}
```
