# trait::extra_item: an impl defines an item the trait does not declare

An `impl Trait for T` block contains a method, associated type or constant
whose name is not one of the trait's.

A trait impl may only define the methods, associated types and constants
that the trait declares. An extra one could never be used through the
trait, and it is usually a typo in a name or an item that was meant for an
inherent impl.

## Erroneous example

```lunar
trait Speak {
    fn speak(&self): str;
}

struct Dog;

impl Speak for Dog {
    fn speak(&self): str {
        return "woof";
    }

    fn wag(self): str {
        return "wag";
    }
}

fn main() {}
```

## Fixed

Move `wag` into an inherent `impl Dog` block (or add it to the trait):

```lunar
trait Speak {
    fn speak(&self): str;
}

struct Dog;

impl Speak for Dog {
    fn speak(&self): str {
        return "woof";
    }
}

impl Dog {
    fn wag(self): str {
        return "wag";
    }
}

fn main() {
    let d = Dog;
    println("{d.speak()} {d.wag()}");
}
```
