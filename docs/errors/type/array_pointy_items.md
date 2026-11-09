# type::array_pointy_items: an array or slice has an item type that is not `Dull`

Every array and slice type, `[T; N]`, `[T; _]`, `[T]`, `[T; ..+]`,
`[T; -..+]`, `[T; &]` and `[T; &mut]`, requires its item type `T` to be
`Dull`. A type is Dull when it is not, and does not contain, a pointer, a
slice, a string or an array. `str`, `String`, `[u8; 4]` and any struct holding
one of those are *pointy*, so they cannot be array items.

The rule comes from the memory model. Every owning array lives on its own
data stack, and views into it stay valid only while the compiler can track
what they point into. Arrays of Dull items are plain blocks of bytes: they can
be copied, grown and shrunk without any per-item bookkeeping, and no item can
own or point into other memory.

The check applies wherever such a type is written: in `let` annotations,
struct fields, function signatures and generic code. For a generic parameter
`T`, the compiler only knows `T` is Dull if it is bounded by `T: Dull`.

## Erroneous example

```lunar
fn count::<T>(items: [T; &]): usize {
    return items.len();
}

fn main() {
    let a: [i32; 3] = [1, 2, 3];
    println("{count(a[..])}");
}
```

## Fixed

Add `Dull` to the generic parameter's bounds:

```lunar
fn count::<T: Dull>(items: [T; &]): usize {
    return items.len();
}

fn main() {
    let a: [i32; 3] = [1, 2, 3];
    println("{count(a[..])}");
}
```
