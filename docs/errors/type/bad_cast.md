# type::bad_cast: an `as` cast between types that cannot be converted

`as` only converts between a fixed set of types:

* any integer or float type to any other integer or float type;
* `bool` and `char` to an integer type;
* `u8` to `char`;
* a raw pointer to another raw pointer type, or to `usize`/`isize`, and
  `usize`/`isize` to a raw pointer;
* a function pointer to a raw pointer.

A cast to the same type is also allowed. Everything else, such as integer to
`bool`, `u32` to `char`, or anything involving strings, arrays or structs, is
rejected. These conversions are either lossy in a way `as` cannot express or
need a check: compare with `!= 0` to get a `bool`, and use `char::from_u32`,
which returns an `Option::<char>`, for code points wider than `u8`.

## Erroneous example

```lunar
fn main() {
    let flags: u8 = 1;
    let on = flags as bool;
    println("{on}");
}
```

## Fixed

Compare instead of casting:

```lunar
fn main() {
    let flags: u8 = 1;
    let on = flags != 0;
    println("{on}");
}
```
