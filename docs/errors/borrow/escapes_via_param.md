# borrow::escapes_via_param: a view is stored into a parameter

A slice or `str` was assigned to a parameter (or a field of one), handing a
view back to the caller through an in-out parameter.

A `mut` parameter is in-out: whatever is stored in it is visible to the
caller after the call. The caller's variable may live much longer than
anything the callee can point at, so a view stored there could outlive the
data it points into. The compiler rejects any assignment of a view into a
parameter when the view points into a value it can track, such as a local
variable of the callee.

## Erroneous example

```lunar
fn pick_name(mut out: str) {
    let name: String = String::from("lunar");
    out = name.as_str();
}

fn main() {
    let mut s: str = "";
    pick_name(s);
    println("{s}");
}
```

## Fixed

Pass an owning value in-out and write into it, so the caller owns the
result:

```lunar
fn pick_name(mut out: String) {
    out.push_str("lunar");
}

fn main() {
    let mut s = String::new();
    pick_name(s);
    println("{s}");
}
```
