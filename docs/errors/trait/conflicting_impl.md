# trait::conflicting_impl: two impls of the same trait overlap

Two `impl` blocks implement the same trait for the same type, or for types
that overlap (for example a blanket `impl::<T: Dull> Trait for T` and an
`impl Trait for i32`).

Trait method calls are resolved statically: for each call the compiler must
pick exactly one impl. If two impls apply to the same type, there is no
single answer, so implementations of a trait must not overlap.

This code is reserved for that rule. The current compiler does not report it
yet: overlapping impls are accepted and the first matching impl is used. Do
not rely on that; keep impls disjoint.

## Erroneous example

```lunar,ignore
trait Speak {
    fn speak(self): i32;
}

struct Dog;

impl Speak for Dog {
    fn speak(self): i32 {
        return 1;
    }
}

impl Speak for Dog {
    fn speak(self): i32 {
        return 2;
    }
}

fn main() {}
```

## Fixed

Keep a single impl of `Speak` for `Dog`:

```lunar
trait Speak {
    fn speak(self): i32;
}

struct Dog;

impl Speak for Dog {
    fn speak(self): i32 {
        return 1;
    }
}

fn main() {
    let d = Dog;
    println("{d.speak()}");
}
```
