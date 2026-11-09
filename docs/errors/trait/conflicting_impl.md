# trait::conflicting_impl: two impls of the same trait overlap

Two `impl` blocks implement the same trait for the same type, or for types
that overlap (for example a blanket `impl::<T: Dull> Trait for T` and an
`impl Trait for i32`).

Trait method calls are resolved statically: for each call the compiler must
pick exactly one impl. If two impls apply to the same type, there is no
single answer, so implementations of a trait must not overlap.

Two impls overlap when their headers can describe the same type. Bounds are
taken into account only when they are definitely false: a blanket
`impl::<T: Dull> Trait for [T; 4]` overlaps with `impl Trait for [i32; 4]`
(`i32` is Dull), but `impl::<T: Dull> Trait for (T, String)` does not overlap
with `impl Trait for (String, String)`, because `String` is not Dull.

"Not Dull" only keeps impls apart when it cannot change behind your back.
That holds for types that are pointy by definition (strings, arrays,
slices, pointers) and for types defined in the same crater as the impl
written for them, since you control their fields. A type from another
crater, such as `Option::<String>` from `std` or a type from a library,
might drop its last pointer or array field in a later version and become
Dull, so an impl for it conflicts with a blanket `impl::<T: Dull>`.

## Erroneous example

```lunar
trait Speak {
    fn speak(&self): i32;
}

struct Dog;

impl Speak for Dog {
    fn speak(&self): i32 {
        return 1;
    }
}

impl Speak for Dog {
    fn speak(&self): i32 {
        return 2;
    }
}

fn main() {}
```

## Fixed

Keep a single impl of `Speak` for `Dog`:

```lunar
trait Speak {
    fn speak(&self): i32;
}

struct Dog;

impl Speak for Dog {
    fn speak(&self): i32 {
        return 1;
    }
}

fn main() {
    let d = Dog;
    println("{d.speak()}");
}
```
