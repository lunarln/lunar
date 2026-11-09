# type::generic_count: the wrong number of generic arguments is supplied

A generic item was given a different number of `::<...>` arguments than it
declares generic parameters. This applies to generic structs, enums and type
aliases in types (`Pair::<i32, i32>`), to traits in bounds and impls, and to
explicit arguments on function and method calls (`size_of::<T>()`).

When the arguments are left out entirely inside a function body, the compiler
infers them. In signatures and other item positions a generic type must
always be written with all its arguments.

The same code is reported for generic arguments on things that take none:
primitive types such as `i32::<u8>`, and generic parameters such as
`T::<i32>`.

## Erroneous example

```lunar
struct Pair::<T> {
    a: T,
    b: T,
}

fn first(p: Pair::<i32, i32>): i32 {
    return p.a;
}

fn main() {
    println("{first(Pair { a: 1, b: 2 })}");
}
```

## Fixed

`Pair` has one type parameter, so it takes one argument:

```lunar
struct Pair::<T> {
    a: T,
    b: T,
}

fn first(p: Pair::<i32>): i32 {
    return p.a;
}

fn main() {
    println("{first(Pair { a: 1, b: 2 })}");
}
```
