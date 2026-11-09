# trait::negative_impl: negative impl of a trait that is not an auto trait

An `impl !Trait for T` was written for an ordinary trait.

A negative impl states that a type does *not* implement a trait. That is
only meaningful for auto traits, which the compiler implements for types
automatically and which a negative impl could opt out of. For an ordinary
trait a type implements it only if there is an `impl`, so not writing one
already says everything a negative impl would.

The only auto trait in Lunar is `Dull`, and it cannot be negated either
(that is `err(trait::dull_impl)`), so in practice every negative impl is an
error.

## Erroneous example

```lunar
trait Speak {
    fn speak(self): str;
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
    fn speak(self): str;
}

struct Rock;

fn main() {}
```
