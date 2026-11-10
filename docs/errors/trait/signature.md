# trait::signature: an impl method's signature differs from the trait's

A method in an `impl Trait for T` block does not have the signature the
trait declares for it. The compiler compares, after substituting `Self` and
the trait's generic arguments:

* the number of the method's own generic parameters;
* the number of parameters (including `self`);
* each parameter's type and mode: taking ownership (`x: T`), borrowing
  (`x: &T`) or in-out (`x: &mut T`), including `+`;
* the return type.

Callers that go through the trait rely on the trait's signature: what they
pass, whether it is moved into the call or changed in place, and what they
get back. An impl with a different signature would break those calls, so
the two must match exactly. In particular `&self` in the trait requires
`&self` in the impl, not `self` or `&mut self`, and `out: &mut +String` in
`Display::fmt` requires `out: &mut +String`, since callers keep their views
into a `+` argument across the call. Only `mut` on a parameter
that takes ownership may differ: `mut x: T` matches `x: T`, since it only
lets the function change its own value.

## Erroneous example

```lunar
trait Area {
    fn area(&self): f64;
}

struct Square {
    side: f64,
}

impl Area for Square {
    fn area(&self): f32 {
        return 1.0;
    }
}

fn main() {}
```

## Fixed

Make the impl's return type `f64`, as the trait declares:

```lunar
trait Area {
    fn area(&self): f64;
}

struct Square {
    side: f64,
}

impl Area for Square {
    fn area(&self): f64 {
        return self.side * self.side;
    }
}

fn main() {
    let s = Square { side: 3.0 };
    println("{s.area()}");
}
```
