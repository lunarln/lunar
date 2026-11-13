# trait::unsafe_impl: `unsafe` missing from an impl of `Send` or `Sync`, or written elsewhere

An `impl Send for T` or `impl Sync for T` was written without `unsafe`, or
`unsafe impl` was written for another trait, an inherent impl or a negative
impl.

`Send` and `Sync` are promises the compiler cannot check: that a value of
the type may move to another thread, and that threads may share views of
one. The compiler works them out from a type's fields, and stops at raw
pointers and cells, which carry no such promise. A type that guards its raw
pointers or cells itself (a lock, an atomic, a handle to memory nobody else
uses) makes the promise with `unsafe impl`, like a call to an `unsafe fn`:
the `unsafe` says the author checked it.

Taking a promise away (`impl !Send for T {}`) is always safe, so negative
impls are not `unsafe`. No other trait is unsafe to implement.

## Erroneous example

```lunar
struct Handle {
    raw: *mut u8,
}

impl Send for Handle {}

fn main() {}
```

## Fixed

```lunar
struct Handle {
    raw: *mut u8,
}

// The memory behind `raw` belongs to one `Handle` at a time.
unsafe impl Send for Handle {}

fn main() {}
```
