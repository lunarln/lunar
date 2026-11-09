# unsafe::in_unsafe_fn: an unsafe operation in an `unsafe fn` without an `unsafe` block

An unsafe operation (calling an `unsafe fn`, an unsafe method or an `extern`
function, dereferencing a raw pointer, or accessing a `static mut`) appears
in the body of an `unsafe fn` but not inside an `unsafe { ... }` block.

Marking a function `unsafe` means only that it is unsafe to *call*: callers
must uphold some condition the compiler cannot check, and must call it from
an `unsafe` block. It does not make the function's own body an unsafe
context. Each unsafe operation in the body still needs its own `unsafe`
block, so that the exact places relying on unchecked conditions stay
visible, even in long unsafe functions.

Outside an `unsafe fn`, the same mistake is `err(unsafe::outside_unsafe)`.

## Erroneous example

```lunar
unsafe fn read(p: *mut i32): i32 {
    return *p;
}

fn main() {
    let mut x: i32 = 7;
    let p = core::intrinsics::addr_of_mut(x);
    let v = unsafe { read(p) };
    println("{v}");
}
```

## Fixed

Put the dereference in an `unsafe` block inside the function:

```lunar
unsafe fn read(p: *mut i32): i32 {
    return unsafe { *p };
}

fn main() {
    let mut x: i32 = 7;
    let p = core::intrinsics::addr_of_mut(x);
    let v = unsafe { read(p) };
    println("{v}");
}
```
