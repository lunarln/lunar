# borrow::missing_marker: a lent argument is passed without `&` or `&mut`

The parameter borrows (`x: &T`) or is in-out (`x: &mut T`), but the
argument is written as if it were moved.

How an argument is passed is written at the call, so that reading a call
tells you what happens to each argument:

| Parameter | Call | What happens |
|---|---|---|
| `x: T` | `f(v)` | `v` is moved (or copied, if it is Dull) |
| `x: &T` | `f(&v)` | `v` is lent; the function can read it |
| `x: &mut T` | `f(&mut v)` | `v` is lent in-out; the function can change it |

No marker is needed when the argument already is a borrow: a borrowed or
in-out parameter of the calling function is passed on as-is (`f(x)` inside
`fn g(x: &T)`), and an in-out parameter can be passed as-is where a borrow
is wanted. A method's receiver also needs no marker: `v.push(1)` lends `v`
to `&mut self` on its own.

## Erroneous example

```lunar
fn bump(x: &mut i32) {
    x += 1;
}

fn main() {
    let mut n = 1;
    bump(n);
    println("{n}");
}
```

## Fixed

```lunar
fn bump(x: &mut i32) {
    x += 1;
}

fn main() {
    let mut n = 1;
    bump(&mut n);
    println("{n}");
}
```
