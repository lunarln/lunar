# trait::ambiguous_assoc: two traits give a type an associated item of the same name

`T::Item` (or `T::N` for a constant) names an associated type (constant)
found through the traits `T` is known to implement: the bounds of a
generic parameter and their supertraits, or the impls of a type. When two of
those traits both have one with that name, `T::Item` could mean either, and
the compiler does not guess.

Name the trait with a qualified path: `<T as Trait>::Item`.

## Erroneous example

```lunar
trait Source {
    type Item;
    fn get(&self): Self::Item;
}

trait Sink {
    type Item;
    fn put(&self, x: Self::Item);
}

fn pass::<T: Source + Sink>(t: &T): T::Item {
    return t.get();
}

fn main() {}
```

## Fixed

`<T as Source>::Item` is the `Item` of `Source`:

```lunar
trait Source {
    type Item;
    fn get(&self): Self::Item;
}

trait Sink {
    type Item;
    fn put(&self, x: Self::Item);
}

fn pass::<T: Source + Sink>(t: &T): <T as Source>::Item {
    return t.get();
}

fn main() {}
```
