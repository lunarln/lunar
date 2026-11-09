# unsafe::outside_unsafe: an unsafe operation outside an `unsafe` block

An unsafe operation was used outside any `unsafe { ... }` block. The unsafe
operations are:

* calling an `unsafe fn`, an unsafe method, or a function declared in an
  `extern` block;
* dereferencing a raw pointer (`*p` where `p` is a `*T` or `*mut T`);
* reading or writing a `static mut`.

The compiler cannot check that these are used correctly: a raw pointer may
be dangling or null, a `static mut` may be accessed from several places at
once, and foreign functions have no Lunar-checked contract. An `unsafe`
block marks the places where the programmer, not the compiler, is
responsible for that, so they are easy to find and review.

Inside an `unsafe fn` the same operations report
`err(unsafe::in_unsafe_fn)` instead.

## Erroneous example

```lunar
static mut COUNTER: u32 = 0;

fn bump() {
    COUNTER += 1;
}

fn main() {
    bump();
}
```

## Fixed

Wrap the access to the `static mut` in an `unsafe` block:

```lunar
static mut COUNTER: u32 = 0;

fn bump() {
    unsafe {
        COUNTER += 1;
    }
}

fn main() {
    bump();
}
```
