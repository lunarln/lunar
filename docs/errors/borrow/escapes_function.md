# borrow::escapes_function: a view of a function's own data escapes the function

A function returned a slice or `str` pointing into data that is freed when
the function returns, or stored such a view in a `static`.

A function's locals are freed when it returns, and so are temporaries.
Parameters passed by value that are Dull (such as a fixed array `[i32; 4]`)
are copies owned by the function, so they are freed too. A view into any of
these would dangle in the caller. Views of slice parameters, of `mut`
(in-out) parameters, and of owning parameters (which are borrowed from the
caller) are fine to return, because their data belongs to the caller.

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

Return the owning value itself, so the caller becomes its owner (or take a
slice parameter and return a view of that):

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
