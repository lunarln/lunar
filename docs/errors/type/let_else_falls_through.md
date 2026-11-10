# type::let_else_falls_through: the `else` block of a `let .. else` finishes

`let pattern = value else { ... };` runs its `else` block when the value does
not match the pattern. The code after the `let` uses the pattern's bindings,
which do not exist in that case, so the `else` block must not finish
normally: it has to leave with `return`, `break` or `continue`, or call a
function that never returns (one whose return type is `!`, such as
`panic`).

## Erroneous example

```lunar
fn first_even(xs: [i32; &]): i32 {
    let Some(x) = find_even(xs) else {
        println("no even number");
    };
    x
}

fn find_even(xs: [i32; &]): Option::<i32> {
    for x in xs {
        if x % 2 == 0 {
            return Some(x);
        }
    }
    None
}

fn main() {
    let xs = [1, 3, 4];
    println("{first_even(xs[..])}");
}
```

## Fixed

The `else` block returns a value from the function instead:

```lunar
fn first_even(xs: [i32; &]): i32 {
    let Some(x) = find_even(xs) else {
        println("no even number");
        return 0;
    };
    x
}

fn find_even(xs: [i32; &]): Option::<i32> {
    for x in xs {
        if x % 2 == 0 {
            return Some(x);
        }
    }
    None
}

fn main() {
    let xs = [1, 3, 4];
    println("{first_even(xs[..])}");
}
```
