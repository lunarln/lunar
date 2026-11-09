# borrow::use_after_move: a value is used after it was moved

A variable was read after the value it held had been moved somewhere else.

Owning values (owning arrays, `String`, and structs, enums or tuples that
contain one) are not copied implicitly. Binding one to another variable,
assigning it, putting it into a struct literal, matching it into a binding
or returning it *moves* it: the new place becomes the owner and the old
variable no longer holds a value. Only one owner may free the data, so the
old variable cannot be used again until it is assigned a new value.

Moves are tracked per field. Moving `p.a` out of a struct leaves `p.b`
usable, but `p` as a whole, and `p.a` itself, can no longer be used. A move
inside one branch of an `if`, `match` or loop counts as a move for the code
that follows, because the compiler cannot know which branch ran.

Passing a value to a function is not a move: parameters are borrowed. Dull
values and views (slices, `str`) are copied, so they never cause this error.

## Erroneous example

```lunar
fn main() {
    let names: [i32; ..+] = [1, 2, 3];
    let saved = names;
    println("{names.len()} {saved.len()}");
}
```

## Fixed

Clone the value if both variables need their own copy (or use only the new
owner after the move):

```lunar
fn main() {
    let names: [i32; ..+] = [1, 2, 3];
    let saved = names.clone();
    println("{names.len()} {saved.len()}");
}
```
