# borrow::shrink_not_owner: only the owner of a value may shrink it

A value was passed to a `!` parameter, or had a shrinking method such as
`pop`, `truncate` or `clear` called on it, from a place that does not own
it.

Shrinking can invalidate views into a value, and only the owner can know
whether such views exist. An in-out parameter (`x: &mut T` or `&mut self`)
is borrowed: the caller owns it and may hold slices into it that this
function cannot see. So shrinking is allowed only on:

* a local variable,
* a parameter that took ownership (`mut x: T` or `mut self`), or
* a parameter declared `x: &mut !T` (or a `&mut !self` receiver), which
  passes the caller's permission along and makes the caller check its own
  views.

The error is reported when the shrunk value is a borrowed parameter not
declared `!`, when it is reached through a slice, and when it has no owner
in the function at all (for example a `static` or data behind a raw
pointer).

## Erroneous example

```lunar
fn drop_last(items: &mut [i64; -..+]) {
    items.pop();
}

fn main() {
    let mut v: [i64; -..+] = [1, 2, 3];
    drop_last(v);
    println("{v.len()}");
}
```

## Fixed

Declare the parameter `&mut !` so callers know it may shrink (or, if the
caller no longer needs the value, take ownership with
`mut items: [i64; -..+]`):

```lunar
fn drop_last(items: &mut ![i64; -..+]) {
    items.pop();
}

fn main() {
    let mut v: [i64; -..+] = [1, 2, 3];
    drop_last(v);
    println("{v.len()}");
}
```
