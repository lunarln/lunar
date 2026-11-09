# lint::unused_variable: a variable is never used

A variable introduced by `let` or by a pattern (in `match`, `if let`,
`while let` or `for`) is never read. This is a warning, not an error.

An unused variable is often a sign of a bug: a value was computed and then
forgotten, or a different variable was used by mistake. If the value really
is not needed, say so by naming the variable with a leading underscore
(`_height`) or using the `_` pattern; such names are not reported.

Function parameters are not checked. The lint runs only for functions that
compiled without errors, and never for the standard library.

## Erroneous example

```lunar
fn main() {
    let width = 3;
    let height = 4;
    println("area: {width * 4}");
}
```

## Fixed

Use the variable (here, `height` was meant instead of the literal `4`):

```lunar
fn main() {
    let width = 3;
    let height = 4;
    println("area: {width * height}");
}
```
