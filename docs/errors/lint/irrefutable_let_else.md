# lint::irrefutable_let_else: a `let .. else` pattern always matches

The pattern of a `let pattern = value else { ... };` matches every value of
its type, so the `else` block can never run. This is a warning, not an
error.

It usually means the `else` block is left over from a change to the value's
type (for example, a function that used to return `Option::<T>` now returns
`T`), or that the pattern is missing the variant it was meant to check.

## Erroneous example

```lunar
fn main() {
    let pair = (1, 2);
    let (a, b) = pair else {
        return;
    };
    println("{a + b}");
}
```

## Fixed

Remove the `else` block:

```lunar
fn main() {
    let pair = (1, 2);
    let (a, b) = pair;
    println("{a + b}");
}
```
