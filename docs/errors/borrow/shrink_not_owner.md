# borrow::shrink_not_owner: only the owner of a value may shrink it

A value was passed to a `!` parameter, or had a shrinking method such as
`pop`, `truncate` or `clear` called on it, from a place that does not own
it.

Shrinking can invalidate views into a value, and only the owner can know
whether such views exist. A plain parameter is borrowed: the caller owns it
and may hold slices into it that this function cannot see. So shrinking is
allowed only on:

* a local variable, or
* a parameter that is itself declared `mut x: !T` (or a `mut self: !Self`
  receiver), which passes the caller's permission along and makes the
  caller check its own views.

The error is reported when the shrunk value is a parameter not declared
`!`, when it is reached through a slice, and when it has no owner in the
function at all (for example a `static` or data behind a raw pointer).

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

Declare the parameter `!` so callers know it may shrink:

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
