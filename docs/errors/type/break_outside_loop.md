# type::break_outside_loop: `break` or `continue` is used where it cannot apply

`break` and `continue` only make sense inside a loop body (`loop`, `while`,
`while let` or `for`). This error is reported when:

* `break` or `continue` appears outside of any loop, for example directly
  in a function body;
* `break value` is used inside a `while` or `for` loop. Only `loop` can
  produce a value with `break`: a `while` or `for` loop can also end because
  its condition fails or its iterator runs out, and then there would be no
  value. Such loops always have type `()`.

To compute a value from a `while` or `for` loop, assign it to a variable
declared before the loop, or rewrite it as a `loop`.

## Erroneous example

```lunar
fn main() {
    let mut i = 0;
    while i < 10 {
        if i * i > 20 {
            break i;
        }
        i += 1;
    }
}
```

## Fixed

The loop is now a `loop`, which can produce a value with `break`:

```lunar
fn main() {
    let mut i = 0;
    let first = loop {
        if i * i > 20 {
            break i;
        }
        i += 1;
    };
    println("{first}");
}
```
