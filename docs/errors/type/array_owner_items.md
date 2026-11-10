# type::array_owner_items: an array or slice has an item type that owns an array or string

Every array and slice type, `[T; N]`, `[T; _]`, `[T]`, `[T; ..+]`,
`[T; -..+]`, `[T; &]` and `[T; &mut]`, requires its item type `T` to be
`Light`: it must not own an array or a string. Dull values (numbers,
structs of numbers) and views (`str`, `[U; &]`, structs holding them) are
Light. `String`, owning arrays such as `[U; ..+]`, and any struct, enum or
tuple holding one are not.

The rule comes from the memory model. Every owning array lives on its own
data stack (a slot of address space), so an array of owners would need one
slot per item and per-item freeing. Items that own nothing are copied as
plain bytes, and the borrow checker tracks what views among them point
into, as it does for a struct with a `str` field. To keep many strings or
arrays, store their data in one place (one `String`, an `Arena`, an
`Interner`) and keep views, `Id`s or `Symbol`s in the array.

The check applies wherever such a type is written: in `let` annotations,
struct fields, function signatures and generic code. For a generic parameter
`T`, the compiler only knows `T` is Light if it is bounded by `T: Light` (or
`T: Dull`, which implies it).

## Erroneous example

```lunar
fn count::<T>(items: &[T; &]): usize {
    return items.len();
}

fn main() {
    let a: [i32; 3] = [1, 2, 3];
    println("{count(a[..])}");
}
```

## Fixed

Add `Light` to the generic parameter's bounds:

```lunar
fn count::<T: Light>(items: [T; &]): usize {
    return items.len();
}

fn main() {
    let a: [i32; 3] = [1, 2, 3];
    println("{count(a[..])}");
}
```
