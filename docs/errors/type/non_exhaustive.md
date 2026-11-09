# type::non_exhaustive: patterns do not cover every possible value

A `match` must handle every value its scrutinee can have. The compiler
checks the arms (ignoring arms with an `if` guard, since a guard can fail)
and reports one value that no arm matches, such as `Dir::East` or
`Some(_)`. Integers, `char` and other types with too many values to list
need a wildcard `_` or binding arm.

The same code is reported for a `let` whose pattern can fail to match
(a *refutable* pattern), such as `let Some(v) = maybe;`. A `let` has no
other branch to take, so its pattern must always match; use `if let`,
`while let` or `match` instead.

## Erroneous example

```lunar
enum Dir {
    North,
    South,
    East,
}

fn name(d: Dir): str {
    return match d {
        Dir::North => "n",
        Dir::South => "s",
    };
}

fn main() {
    println("{name(Dir::East)}");
}
```

## Fixed

An arm for the missing variant was added:

```lunar
enum Dir {
    North,
    South,
    East,
}

fn name(d: Dir): str {
    return match d {
        Dir::North => "n",
        Dir::South => "s",
        Dir::East => "e",
    };
}

fn main() {
    println("{name(Dir::East)}");
}
```
