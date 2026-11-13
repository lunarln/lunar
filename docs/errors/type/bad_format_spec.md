# type::bad_format_spec: a format spec does not suit the value's type

A format hole's spec (the part after the `:`, as in `{x:>8.2}`) is written
in Rust's syntax: `[[fill]align][sign]['#']['0'][width]['.' precision][type]`.
Some of its parts only make sense for some types:

* the types `x`, `X`, `o` and `b` (hexadecimal, octal and binary) show the
  bits of an integer, so the value must be an integer type;
* the types `e` and `E` (scientific notation) are for floats, `f32` and
  `f64`;
* `+` (always show the sign) and `0` (pad with zeros after the sign) are for
  numbers: integers and floats;
* a precision means the digits after the point of a float, or with
  `Display` how many characters of a `str`, `String`, `char` or `bool` to
  keep. It means nothing for an integer, and is accepted there as in Rust.
  For anything else (a struct, an array, a value shown with `?` that is not
  a number) it is an error: `Display` and `Debug` implementations do not
  see the spec, so it could not be honored.

The spec is checked once the value's type is known, so a generic value
(`T: Display`) can be padded (`{x:>8}`) but not given a radix or a
precision. A spec that is malformed (an unknown type, a `.` without a
precision) is a different error: `syntax::bad_format`.

## Erroneous example

```lunar
fn main() {
    let ratio = 0.75;
    println("{ratio:x}");
}
```

## Fixed

`x` shows integers in hexadecimal; for a float, say how many digits to
show, or use scientific notation:

```lunar
fn main() {
    let ratio = 0.75;
    println("{ratio:.3} {ratio:e}");
}
```
