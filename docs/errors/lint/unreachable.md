# lint::unreachable: code that can never run

Code follows a statement that never completes normally, such as `return`,
`break`, `continue`, a loop without a `break`, or a call to a function
returning `!` like `panic`. That code can never execute. This is a warning,
not an error.

Unreachable code usually means a `return` or `break` was left in by mistake
or placed too early, so the code after it was meant to run but silently
does not.

This code is reserved for that lint. The current compiler does not report
it yet; unreachable code is accepted without a warning.

## Erroneous example

```lunar,ignore
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
