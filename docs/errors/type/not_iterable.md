# type::not_iterable: a `for` loop is over something that cannot be iterated

A `for` loop works over:

* integer ranges, `a..b` and `a..=b`;
* arrays and slices of any kind, yielding their (Dull) items by value;
* any type implementing `core::iter::Iterator`, yielding its `Item`s.

This error is reported when the value after `in` is none of these, and when
a range has bounds that are not integers (such as `0.0..1.0`).

`str` and `String` are not iterated directly, because "the items of a
string" could mean bytes or characters. Call `.chars()` or `.bytes()` to
choose.

## Erroneous example

```lunar
fn main() {
    let name = "lunar";
    for c in name {
        println("{c}");
    }
}
```

## Fixed

The loop now iterates the string's characters:

```lunar
fn main() {
    let name = "lunar";
    for c in name.chars() {
        println("{c}");
    }
}
```
