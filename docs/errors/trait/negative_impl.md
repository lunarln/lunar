# trait::negative_impl: negative impl of a trait that is not an auto trait

An `impl !Trait for T` was written for an ordinary trait.

A negative impl states that a type does *not* implement a trait. That is
only meaningful for auto traits, which the compiler implements for types
automatically and which a negative impl could opt out of. For an ordinary
trait a type implements it only if there is an `impl`, so not writing one
already says everything a negative impl would.

The auto traits that can be negated are `Send` and `Sync`: `impl !Send for
Handle {}` keeps a type from moving to another thread even though its fields
could. `Dull`, `Light` and `Detached` are auto traits too, but whether a type
has them is a fact about its fields and cannot be negated (that is
`err(trait::dull_impl)`).

## Erroneous example

```lunar
trait Speak {
    fn speak(&self): str;
}

struct Rock;

impl !Speak for Rock {}

fn main() {}
```

## Fixed

Remove the negative impl. `Rock` does not implement `Speak` because there is
no `impl Speak for Rock`:

```lunar
trait Speak {
    fn speak(&self): str;
}

struct Rock;

fn main() {}
```
