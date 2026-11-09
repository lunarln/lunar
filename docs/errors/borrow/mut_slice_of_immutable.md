# borrow::mut_slice_of_immutable: a mutable slice is taken of an immutable value

A `[T; &mut]` slice would let code write into the elements it covers, so it
may only be taken of a value that is itself mutable: a `let mut` binding, a
`mut` parameter, or a field or element of one.

The compiler does not currently emit this code. Taking a mutable slice of
an immutable value is reported as `err(type::immutable)` instead, and
converting a `[T; &]` slice into a `[T; &mut]` one is
`err(type::mismatch)`. This code is reserved for a future borrow-checker
version of the rule.

## Erroneous example

```lunar,ignore
fn zero(s: [i32; &mut]) {
    s.fill(0);
}

fn main() {
    let nums: [i32; 3] = [1, 2, 3];
    zero(nums[..]);
    println("{nums[0]}");
}
```

## Fixed

Declare the value `let mut`:

```lunar,ignore
fn zero(s: [i32; &mut]) {
    s.fill(0);
}

fn main() {
    let mut nums: [i32; 3] = [1, 2, 3];
    zero(nums[..]);
    println("{nums[0]}");
}
```
