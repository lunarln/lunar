# type::variadic_not_extern: a function that is not extern is variadic

A function declared with `...` after its parameters takes a variable number
of arguments, as C's `printf` does. Lunar only supports this for functions
declared in an `extern "C" { ... }` block, which are implemented in C and
called with the C calling convention. A Lunar function has no way to read
the extra arguments.

To take any number of values in a Lunar function, take a slice.

## Erroneous example

```lunar
fn sum(first: i32, ...): i32 {
    return first;
}

fn main() {
    println("{sum(1, 2, 3)}");
}
```

## Fixed

The values are passed as a slice:

```lunar
fn sum(values: [i32; &]): i32 {
    let mut total = 0;
    for v in values {
        total += v;
    }
    return total;
}

fn main() {
    let values: [i32; 3] = [1, 2, 3];
    println("{sum(values[..])}");
}
```
