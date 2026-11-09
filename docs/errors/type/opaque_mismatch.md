# type::opaque_mismatch: a function returning `impl Trait` returns different types

A function whose return type is `impl Trait` must return one concrete type
from every `return` (and its tail expression): callers are compiled for
that single type. Two closures are always two different types, even when
they are written alike.

The type it returns must also have the bounds the signature promises.

## Erroneous example

```lunar
fn pick(up: bool): impl Fn(i32): i32 {
    if up {
        return move |x| x + 1;
    }
    return move |x| x - 1;
}

fn main() {
    let f = pick(true);
    println("{f(1)}");
}
```

## Fixed

Return one closure, and decide inside it:

```lunar
fn pick(up: bool): impl Fn(i32): i32 {
    return move |x| if up { x + 1 } else { x - 1 };
}

fn main() {
    let f = pick(true);
    println("{f(1)}");
}
```
