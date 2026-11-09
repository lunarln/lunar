# trait::dull_impl: `Dull` implemented or `!`implemented by hand

An `impl Dull for T` or `impl !Dull for T` was written. `core::ptr::Dull` is
implemented by the compiler, never by user code.

A type is Dull exactly when it contains no pointers, slices, strings or
arrays, at any depth. The compiler works this out from the type's fields, so
there is nothing to write by hand. Array and slice items must be Dull, and
Dull values are copied implicitly, so letting code claim `Dull` for a type
that holds a pointer or an owning array would allow copies of values that
must be moved, and arrays of values that may point into other arrays.

Removing `Dull` from a type with `impl !Dull` is not allowed either: whether
a type is Dull is a fact about its layout, not a choice.

## Erroneous example

```lunar
struct Meters {
    value: f64,
}

impl Dull for Meters {}

fn main() {}
```

## Fixed

Delete the impl. `Meters` holds only an `f64`, so it is already Dull and can
be used as an array item:

```lunar
struct Meters {
    value: f64,
}

fn main() {
    let laps: [Meters; 2] = [Meters { value: 400.0 }, Meters { value: 800.0 }];
    println("{laps[1].value}");
}
```
