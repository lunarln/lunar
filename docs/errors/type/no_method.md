# type::no_method: a method or associated function cannot be found

A method call `x.name(...)` found no method of that name for the type of `x`.
The compiler searches the type's inherent `impl` blocks and every trait
implemented for it. On arrays and strings it also *autoslices*: a
`[T; -..+]` receiver finds the methods of `[T; ..+]`, `[T]`, `[T; &mut]` and
`[T; &]`, and a `String` finds the methods of `str`. If the type has a field
with that name, the diagnostic points out that a field is not a method.

For a value of a generic type `T`, only the methods of the traits in `T`'s
bounds are available.

The same code is reported for a path such as `Type::name()` or
`Trait::name()` when the type or trait has no function or associated item
with that name.

## Erroneous example

```lunar
struct Counter {
    n: i32,
}

fn main() {
    let mut c = Counter { n: 0 };
    c.increment();
    println("{c.n}");
}
```

## Fixed

Define the method in an `impl` block for the type:

```lunar
struct Counter {
    n: i32,
}

impl Counter {
    fn increment(mut self) {
        self.n += 1;
    }
}

fn main() {
    let mut c = Counter { n: 0 };
    c.increment();
    println("{c.n}");
}
```
