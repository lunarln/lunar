# trait::signature: an impl method's signature differs from the trait's

A method in an `impl Trait for T` block does not have the signature the
trait declares for it. The compiler compares, after substituting `Self` and
the trait's generic arguments:

* the number of the method's own generic parameters;
* the number of parameters (including `self`);
* each parameter's type, and whether it is `mut` (in-out);
* the return type.

Callers that go through the trait rely on the trait's signature: what they
pass, whether their argument may be changed in place, and what they get
back. An impl with a different signature would break those calls, so the
two must match exactly. In particular `mut self` in the trait requires
`mut self` in the impl, and the other way round.

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
