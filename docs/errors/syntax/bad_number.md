# syntax::bad_number: a numeric literal is malformed

A number in the source could not be read. This happens when:

* an integer contains a digit that is invalid for its base, or an unknown
  suffix (`0b102`, `0xFG`, `100u31`);
* a base prefix has no digits after it (`0x`);
* an integer does not fit in 128 bits;
* a float literal has an integer suffix (`1.5u8`) or is otherwise invalid;
* a tuple field index is not a plain number (`t.0x1`).

Valid integer suffixes are `i8 i16 i32 i64 isize u8 u16 u32 u64 usize`, and
valid float suffixes are `f32 f64`. Underscores between digits are allowed
(`1_000`).

Whether a well-formed literal fits its type (for example `300u8`) is checked
later and reported as `type::literal_range`.

## Erroneous example

```lunar
fn main() {
    let limit = 100u31;
    println("{limit}");
}
```

## Fixed

The suffix names an existing integer type:

```lunar
fn main() {
    let limit = 100u32;
    println("{limit}");
}
```
