# name::not_a_trait: a trait was expected, but the name refers to something else

The name was found, but it is not a trait. Traits are expected in generic
bounds (`T: Bound`), in supertrait lists (`trait A: B`) and after `impl` in
`impl Trait for Type`. A struct, enum, type alias or module cannot appear
there.

A common cause is a type and a trait with similar names, or writing
`impl Type for Other` when an inherent `impl Type { ... }` was meant.

## Erroneous example

```lunar
struct Printable;

fn show::<T: Printable>(x: T) {}

fn main() {
    show(1);
}
```

## Fixed

The bound names a trait:

```lunar
trait Printable {
    fn print(self);
}

impl Printable for i32 {
    fn print(self) {
        println("{self}");
    }
}

fn show::<T: Printable>(x: T) {
    x.print();
}

fn main() {
    show(1);
}
```
