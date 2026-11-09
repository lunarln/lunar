# name::private: an item or field is not visible from here

Items are private to the module that defines them (and that module's child
modules) unless they are marked `pub`. Struct fields are private to the
struct's module unless they are marked `pub`.

This code is reported when:

* a path reaches a non-`pub` item of another module, as in
  `shapes::unit()` where `unit` is not `pub`;
* code outside the struct's module reads or assigns a private field
  (`c.radius`);
* code outside the struct's module names a private field in a struct
  literal.

Enum variant fields have no visibility of their own: they are as visible as
the enum. Making a struct `pub` does not make its fields public; provide a
`pub` constructor function, or mark the fields `pub`.

## Erroneous example

```lunar
mod shapes {
    pub struct Circle { radius: f64 }
}

fn main() {
    let c = shapes::Circle { radius: 2.0 };
    println("{c.radius}");
}
```

## Fixed

The field is marked `pub`:

```lunar
mod shapes {
    pub struct Circle { pub radius: f64 }
}

fn main() {
    let c = shapes::Circle { radius: 2.0 };
    println("{c.radius}");
}
```
