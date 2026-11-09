# type::fmtargs_outside_param: `FmtArgs` is used somewhere other than a parameter

`FmtArgs` is the type of an interpolated string literal. A string literal
only becomes `FmtArgs` at the moment it is passed to a parameter of that
type, such as the one of `println`, `print`, `panic` or `String::write`; the
`{expr}` pieces are evaluated in the caller. It is not a value that can be
stored, so it cannot be the type of a `let`, a struct field, a return type or
part of another type.

To keep formatted text around, build a `String` with `String::write` or
`core::fmt::format`. To forward formatting to another function, give that
function a `FmtArgs` parameter of its own.

## Erroneous example

```lunar
fn main() {
    let name = "moon";
    let greeting: FmtArgs = "hello, {name}";
    println(greeting);
}
```

## Fixed

Format into a `String`, then print that:

```lunar
fn main() {
    let name = "moon";
    let mut greeting = String::new();
    greeting.write("hello, {name}");
    println("{greeting}");
}
```
