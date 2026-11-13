# name::not_a_type: a type was expected, but the name refers to something else

The name was found, but it is not a type, so it cannot be used where a type
is expected. This is reported for:

* a module or trait used as a type, as in `fn f(s: Shape)` where `Shape` is
  a trait. Lunar has no trait objects; take a generic parameter bounded by
  the trait instead;
* an associated type of a type that is not a generic parameter or `Self`,
  such as `Point::Item`: which trait's `Item` is meant depends on the trait,
  so write `<Point as Trait>::Item` (for a generic parameter, `T::Item` is
  found through its bounds; see `name::no_assoc`);
* a path through something that is not a type, trait or module, such as
  `std::io::println::Item`;
* a struct literal or struct pattern whose path does not name a struct or an
  enum variant with fields: a module, an enum itself (`E { .. }` instead of
  `E::Variant { .. }`), a type alias of a non-struct type, or `Self` inside
  an enum's `impl`.

## Erroneous example

```lunar
trait Shape {
    fn area(&self): f64;
}

struct Square { side: f64 }

impl Shape for Square {
    fn area(&self): f64 {
        return self.side * self.side;
    }
}

fn print_area(s: Shape) {
    println("{s.area()}");
}

fn main() {
    print_area(Square { side: 2.0 });
}
```

## Fixed

The function takes a generic type bounded by the trait:

```lunar
trait Shape {
    fn area(&self): f64;
}

struct Square { side: f64 }

impl Shape for Square {
    fn area(&self): f64 {
        return self.side * self.side;
    }
}

fn print_area::<S: Shape>(s: &S) {
    println("{s.area()}");
}

fn main() {
    print_area(&Square { side: 2.0 });
}
```
