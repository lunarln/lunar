# name::unbound_in_alternative: an alternative of a `|` pattern does not bind a variable the others bind

A `|` pattern matches if any of its alternatives does, and the code after
it (a `match` arm, the body of an `if let`, the rest of the block after a
`let`) cannot know which one matched. So every alternative must bind the
same variables: a variable that one alternative binds and another leaves out
would have no value when the other one matched.

The error points at the alternative that leaves the variable out and at a
place where it is bound. Alternatives nested inside another pattern
(`Some(A(x) | B(x))`) follow the same rule, and so does the `|` pattern
around them.

To fix it, bind the variable in every alternative (from whichever field
holds it there), or match the alternatives in separate arms.

## Erroneous example

```lunar
enum Reading {
    Exact(i32),
    Range(i32, i32),
    Missing,
}

fn low(r: Reading): i32 {
    return match r {
        Reading::Exact(v) | Reading::Range(v, _) | Reading::Missing => v,
    };
}

fn main() {
    println("{low(Reading::Range(3, 9))}");
}
```

## Fixed

`Missing` has no value to bind, so it gets an arm of its own:

```lunar
enum Reading {
    Exact(i32),
    Range(i32, i32),
    Missing,
}

fn low(r: Reading): i32 {
    return match r {
        Reading::Exact(v) | Reading::Range(v, _) => v,
        Reading::Missing => 0,
    };
}

fn main() {
    println("{low(Reading::Range(3, 9))}");
}
```
