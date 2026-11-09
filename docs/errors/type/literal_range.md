# type::literal_range: an integer literal does not fit in its type

An integer literal has a value outside the range of the integer type it is
given, either from a suffix (`300u8`), an annotation, or inference. For
example `u8` holds 0 to 255 and `i8` holds -128 to 127; the error's note
shows the range of the type. A negative literal such as `-129` is checked
as a whole.

Literals are never silently truncated or wrapped. The same code is reported,
once, for a literal larger than any integer type can hold (above
`18446744073709551615`, the largest `u64`).

A negated literal of an unsigned type, as in `let n: u32 = -1;`, is
`type::bad_operand` ("cannot negate unsigned integer `u32`").

Use a wider type, or write a value that fits.

## Erroneous example

```lunar
fn main() {
    let level: u8 = 300;
    println("{level}");
}
```

## Fixed

The variable now has a type wide enough for the value:

```lunar
fn main() {
    let level: u16 = 300;
    println("{level}");
}
```
