# syntax::bad_attribute: an attribute is unknown or malformed

An attribute on an item is not one the compiler understands, or it is used
incorrectly. This code is reported for:

* an unknown attribute, or a known one in the wrong form. The accepted
  attributes are `#[cfg(...)]`, `#[test]`, `#[inline]`, `#[lang = "..."]`,
  `#[intrinsic]`, `#[link_name = "..."]`, `#[allow(...)]`, `#[doc = "..."]`
  (also written as a `///` comment), `#[derive(...)]`, `#[derive_macro]`
  and `#[export]`;
* `#[cfg(...)]` with zero or several predicates (combine them with `all`,
  `any` or `not` instead);
* an unknown predicate function inside `cfg`, such as `cfg(foo(x))`;
* `#[derive(...)]` on something other than a struct or enum, or with
  something other than names inside;
* `#[export]` on an item written by hand: it only makes sense on an item a
  macro defines (see `docs/design/macros.md`);
* an `extern` block with an ABI other than `"C"`.

An unknown `cfg` name or value such as `#[cfg(linux)]` is not an error: it
is false, so the item is compiled out, and the compiler warns about it with
`lint::unknown_cfg`. Use `#[cfg(target_os = "linux")]`.

## Erroneous example

```lunar
#[must_use]
fn double(x: i32): i32 {
    return x * 2;
}

fn main() {
    println("{double(2)}");
}
```

## Fixed

Lunar has no `must_use`; the attribute is removed:

```lunar
fn double(x: i32): i32 {
    return x * 2;
}

fn main() {
    println("{double(2)}");
}
```
