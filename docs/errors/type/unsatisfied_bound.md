# type::unsatisfied_bound: a type does not implement a trait that is required

A generic function, method or type was used with a type argument that does
not implement one of the traits in its bounds. For a call like
`largest::<T: Ord + Dull>(...)`, every `T` it is instantiated with must
implement `Ord` and `Dull`. Inside generic code, a type parameter only
satisfies the bounds that are written on it, so the fix is often to add the
missing trait to the caller's own bounds.

Trait calls are resolved statically and monomorphized, so the compiler must
be able to pick an implementation for every concrete type at compile time.

The same code is reported for an `impl` of a trait whose supertraits the type
does not implement: `impl Ord for T` requires an `impl Eq for T` as well,
because `Ord: Eq`.

## Erroneous example

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn show::<T: Display>(value: &T) {
    println("{value}");
}

fn main() {
    show(Point { x: 1, y: 2 });
}
```

## Fixed

Implement the required trait for the type:

```lunar
struct Point {
    x: i32,
    y: i32,
}

impl Display for Point {
    fn fmt(&self, out: &mut String) {
        out.write("({self.x}, {self.y})");
    }
}

fn show::<T: Display>(value: &T) {
    println("{value}");
}

fn main() {
    show(Point { x: 1, y: 2 });
}
```
