# trait::dull_impl: `Dull`, `Light` or `Detached` implemented or `!`implemented by hand

An `impl Dull for T`, `impl Light for T`, or a negative impl of either was
written. `core::ptr::Dull` and `core::ptr::Light` are implemented by the
compiler, never by user code.

A type is Dull exactly when it contains no pointers, slices, strings or
arrays, at any depth, and Light when it owns no array or string. The
compiler works both out from the type's fields, so there is nothing to
write by hand. Array and slice items must be Light, and Light values are
copied implicitly, so letting code claim `Light` for a type that owns an
array would allow copies of values that must be moved, and arrays of
owners.

Removing `Dull` from a type with `impl !Dull` is not allowed either: whether
a type is Dull is a fact about its layout, not a choice.

The same goes for `core::marker::Detached` (the type holds no views): it
follows from the fields, and claiming it for a type holding a `str` would let
that view outlive the text it points into. (`Send` and `Sync` are different:
a type may promise them with `unsafe impl`, or give them up with a negative
impl.)

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
