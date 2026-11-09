# borrow::use_after_move: a value is used after it was moved

A variable was read after the value it held had been moved somewhere else.

Owning values (owning arrays, `String`, and structs, enums or tuples that
contain one) are not copied implicitly. Binding one to another variable,
assigning it, putting it into a struct literal, matching it into a binding,
returning it, or passing it to a parameter that takes ownership (`x: T`, or
a `self` receiver) *moves* it: the new place becomes the owner and the old
variable no longer holds a value. Only one owner may free the data, so the
old variable cannot be used again until it is assigned a new value.

Call sites look the same for every parameter mode, so `f(s)` moves `s` only
if `f` declares the parameter `s: String` rather than `s: &String` or
`s: &mut String`.

Moves are tracked per field. Moving `p.a` out of a struct leaves `p.b`
usable, but `p` as a whole, and `p.a` itself, can no longer be used. A move
inside one branch of an `if`, `match` or loop counts as a move for the code
that follows, because the compiler cannot know which branch ran.

Dull values and views (slices, `str`) are copied, so they never cause this
error.

## Erroneous example

```lunar
fn shout(mut s: String): String {
    s.push_str("!");
    return s;
}

fn main() {
    let name = String::from("lunar");
    let loud = shout(name);
    println("{name} {loud}");
}
```

## Fixed

Borrow the argument if the function only needs to read it, so the caller
keeps its value (or pass `name.clone()`, or use only the new owner after
the move):

```lunar
fn shout(s: &String): String {
    let mut loud = s.clone();
    loud.push_str("!");
    return loud;
}

fn main() {
    let name = String::from("lunar");
    let loud = shout(name);
    println("{name} {loud}");
}
```
