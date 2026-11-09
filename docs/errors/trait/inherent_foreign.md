# trait::inherent_foreign: inherent methods on a type from another crate

An inherent `impl T { ... }` block (one without a trait) was written for a
type that is not defined in the current crate: a primitive such as `i32`,
an array or slice type such as `[i32; ..+]`, or a type from `core` or `std`
such as `String` or `Option::<i32>`.

Inherent methods belong to the crate that defines the type. If any crate
could add them, two crates could define the same method name on the same
type, and the methods a type has would depend on which crates happen to be
compiled together.

To add methods to a foreign type, declare a trait with those methods and
implement it for the type. Trait impls for foreign types are allowed.

## Erroneous example

```lunar
impl i32 {
    fn double(self): i32 {
        return self * 2;
    }
}

fn main() {}
```

## Fixed

Put the method in a trait and implement the trait for `i32`:

```lunar
trait Double {
    fn double(self): i32;
}

impl Double for i32 {
    fn double(self): i32 {
        return self * 2;
    }
}

fn main() {
    let n: i32 = 21;
    println("{n.double()}");
}
```
