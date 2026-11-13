# trait::auto_impl: `Send` or `Sync` implemented for a type of another crater, or not a struct or enum

An impl of `Send` or `Sync` (positive or negative) names a type that is not a
struct or enum defined in the same crater: a raw pointer, a slice, a tuple,
or a type from a library.

Whether a type is `Send` or `Sync` follows from its parts. Only the crater
that defines a struct or enum may change that, because only it knows what
its private fields are for. Built-in types cannot be changed at all: a raw
pointer is neither Send nor Sync wherever it appears.

To send something that holds a raw pointer, wrap it in a struct of your own
and implement the trait for that struct.

## Erroneous example

```lunar
unsafe impl Send for *mut u8 {}

fn main() {}
```

## Fixed

```lunar
struct Block {
    start: *mut u8,
}

// A `Block` is the only owner of the memory at `start`.
unsafe impl Send for Block {}

fn main() {}
```
