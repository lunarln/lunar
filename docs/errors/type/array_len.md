# type::array_len: an array has the wrong length

The length of a fixed array is part of its type: `[u8; 3]` and `[u8; 2]` are
different types. This error is reported when:

* an array literal `[a, b, ...]` has a different number of elements than the
  annotated fixed length;
* a repeat expression `[value; n]` produces a different number of elements
  than the annotated fixed length;
* an array literal does not match an inferred length `[T; _]` that was
  already fixed by another use;
* the length in an array type, such as `[T; N]`, is a constant that is not a
  non-negative integer.

Owning arrays (`[T]`, `[T; ..+]`, `[T; -..+]`) have no length in their type,
so any number of elements is accepted for them.

## Erroneous example

```lunar
fn main() {
    let rgb: [u8; 3] = [255, 128];
    println("{rgb[0]}");
}
```

## Fixed

The literal now has exactly as many elements as the type says:

```lunar
fn main() {
    let rgb: [u8; 3] = [255, 128, 0];
    println("{rgb[0]}");
}
```
