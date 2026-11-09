# borrow::escapes_function: a view of a function's own data escapes the function

A function returned a slice or `str` pointing into data that is freed when
the function returns, or stored such a view in a `static`.

A function's locals are freed when it returns, and so are temporaries. So
are parameters that take ownership (`x: T` or `mut x: T`): the caller's
value was moved into the call (or copied, for a Dull type such as a fixed
array `[i32; 4]`), and the function frees it on return. A view into any of
these would dangle in the caller.

Views of slice parameters, of borrowed owning values (`x: &T`) and of in-out
parameters (`x: &mut T`) are fine to return, because their data belongs to
the caller.

Storing a view of a local value in a `static` is rejected for the same
reason: the static outlives every function call.

## Erroneous example

```lunar
fn evens(n: i32): [i32; &] {
    let mut all: [i32; ..+] = [];
    for i in 0..n {
        all.push(i * 2);
    }
    return all[..];
}

fn main() {
    let e = evens(4);
    println("{e.len()}");
}
```

## Fixed

Return the owning value itself, so the caller becomes its owner (or borrow
the data from the caller with a slice or `&T` parameter and return a view of
that):

```lunar
fn evens(n: i32): [i32; ..+] {
    let mut all: [i32; ..+] = [];
    for i in 0..n {
        all.push(i * 2);
    }
    return all;
}

fn main() {
    let e = evens(4);
    println("{e.len()}");
}
```
