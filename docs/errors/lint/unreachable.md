# lint::unreachable: code that can never run

Code follows a statement that never completes normally, such as `return`,
`break`, `continue`, a loop without a `break`, a call to a function
returning `!` like `panic`, or an `if` or `match` all of whose branches do
one of those. That code can never execute. This is a warning, not an
error.

The warning is reported once per block, at the first unreachable statement
(or the block's final expression), with a second label on the statement
that never finishes. Like the other lints, it is only reported for
functions that compiled without errors.

Unreachable code usually means a `return` or `break` was left in by mistake
or placed too early, so the code after it was meant to run but silently
does not.

## Erroneous example

```lunar
fn total(a: i32, b: i32): i32 {
    return a;
    let sum = a + b;
    return sum;
}

fn main() {
    println("{total(1, 2)}");
}
```

## Fixed

Remove the early `return`, so the rest of the function runs:

```lunar
fn total(a: i32, b: i32): i32 {
    let sum = a + b;
    return sum;
}

fn main() {
    println("{total(1, 2)}");
}
```
