# trait::not_dyn_compatible: a trait that cannot be used as `dyn Trait`

A `dyn Trait` names a trait whose methods cannot all be called through a
trait object.

A `dyn Trait` is a view of a value of some type that implements the trait,
with a table of that type's methods (its vtable). A call through it finds
the method in the table and passes it the viewed value as `self`. That only
works for methods that

* take `&self`, `&mut self` or `&mut +self` (a method without `self` has no
  value to find the table through, and one taking `self` by value would
  take the value out of a view);
* have no generic parameters of their own (a table holds one function per
  method, not one per type argument);
* mention `Self` nowhere else in their signature (the caller of a `dyn`
  does not know which type `Self` is).

The supertraits' methods are in the table too, so they must follow the same
rules. Methods with a default body are exempt: they are not in the table,
and a call of one on a `dyn` runs the default body with `Self` being the
`dyn` type itself. Closure traits (`Fn`, `FnMut`, `FnOnce`) cannot be used
as `dyn` yet.

## Erroneous example

```lunar
trait Shape {
    fn area(&self): f64;
    fn unit(): Self;
}

fn total(shapes: [dyn Shape; &]): f64 {
    let mut t = 0.0;
    for s in shapes {
        t += s.area();
    }
    return t;
}

fn main() {}
```

## Fixed

Move what does not fit a vtable into another trait (or give it a default
body):

```lunar
trait Shape {
    fn area(&self): f64;
}

trait Unit {
    fn unit(): Self;
}

fn total(shapes: [dyn Shape; &]): f64 {
    let mut t = 0.0;
    for s in shapes {
        t += s.area();
    }
    return t;
}

fn main() {}
```
