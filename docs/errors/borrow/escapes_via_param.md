# borrow::escapes_via_param: a view is stored into an in-out parameter

A slice or `str` was assigned to an in-out parameter (or a field of one),
handing a view back to the caller.

An `x: &mut T` parameter is the caller's own variable: whatever is stored in
it is visible to the caller after the call. The caller's variable may live
much longer than anything the callee can point at, so a view stored there
could outlive the data it points into. The compiler rejects any assignment
of a view into an in-out parameter when the view points into a value it can
track, such as a local variable of the callee.

Parameters that take ownership (`mut x: T`) are the function's own values,
so storing a view in one follows the rules for local variables instead.

## Erroneous example

```lunar
fn pick_name(out: &mut str) {
    let name: String = String::from("lunar");
    out = name.as_str();
}

fn main() {
    let mut s: str = "";
    pick_name(&mut s);
    println("{s}");
}
```

## Fixed

Pass an owning value in-out and write into it, so the caller owns the
result:

```lunar
fn pick_name(out: &mut String) {
    out.push_str("lunar");
}

fn main() {
    let mut s = String::new();
    pick_name(&mut s);
    println("{s}");
}
```
