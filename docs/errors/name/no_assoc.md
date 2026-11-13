# name::no_assoc: no associated type or constant of that name

An associated type or constant was named, but there is none of that name
where it was looked for:

* `T::Item` for a generic parameter `T` (or `Self`) looks through the traits
  of `T`'s bounds, and their supertraits;
* a constraint `Trait::<Item = X>` and a qualified path `<X as
  Trait>::Item` look in `Trait` and its supertraits;
* `<X as Trait>::NAME` as a value looks for a constant (or a method) of
  `Trait`.

The name may be misspelled, or `T` may lack the bound that brings it.

## Erroneous example

```lunar
fn count::<I: Iterator::<Itme = u8>>(it: I): usize {
    return it.count();
}

fn main() {}
```

## Fixed

`Iterator`'s associated type is `Item`:

```lunar
fn count::<I: Iterator::<Item = u8>>(it: I): usize {
    return it.count();
}

fn main() {
    println("{count("ab".bytes())}");
}
```
