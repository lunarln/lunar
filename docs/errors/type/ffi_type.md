# type::ffi_type: a type cannot be used with the C ABI

Functions declared in an `extern "C" { ... }` block are called with the C
calling convention, so their parameters and return types must have a C
equivalent. Only integers, floats, `bool`, `()`, `!`, raw pointers (`*T`,
`*mut T`) and function pointers are allowed.

Lunar types such as `str`, slices, owning arrays, `String`, structs and
enums have no defined C layout. Pass a pointer and a length instead; `str`
and slices provide them with `as_bytes()`, `as_ptr()` and `len()`.

A non-extern function declared variadic (`fn f(x: i32, ...)`) is a
different error: `type::variadic_not_extern`.

## Erroneous example

```lunar
extern "C" {
    fn puts(s: str): i32;
}

fn main() {}
```

## Fixed

The extern function now takes a raw pointer to the bytes, as C expects:

```lunar
extern "C" {
    fn puts(s: *u8): i32;
}

fn main() {}
```
