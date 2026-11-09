# lint::unused_mut: a `mut` variable is never mutated

A variable was declared with `let mut` (or `mut` in a pattern) but is never
changed. This is a warning, not an error.

A variable counts as mutated when it, or a field or element of it, is
assigned to (`=`, `+=`, ...), passed to a `mut` (in-out) parameter or `mut
self` method such as `push`, or mutably sliced (`[T; &mut]`). Reading it,
taking a read-only slice, or passing it to a plain parameter does not count.

Keeping bindings immutable unless they need to change makes code easier to
follow: a reader knows a plain `let` keeps its value. An unneeded `mut` can
also hide a bug where an update was meant to happen but was forgotten.

Variables whose names start with `_` are not reported. The lint runs only
for functions that compiled without errors, and never for the standard
library.

## Erroneous example

```lunar
fn main() {
    let mut total = 0;
    let n = 5;
    println("{total + n}");
}
```

## Fixed

Remove the `mut`:

```lunar
fn main() {
    let total = 0;
    let n = 5;
    println("{total + n}");
}
```
