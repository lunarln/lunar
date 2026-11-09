# syntax::bad_attribute: an attribute is unknown or malformed

An attribute on an item is not one the compiler understands, or it is used
incorrectly. This code is reported for:

* an unknown attribute, or a known one in the wrong form. The accepted
  attributes are `#[cfg(...)]`, `#[test]`, `#[inline]`, `#[lang = "..."]`,
  `#[intrinsic]`, `#[link_name = "..."]`, `#[allow(...)]` and
  `#[doc = "..."]`;
* `#[cfg(...)]` with zero or several predicates (combine them with `all`,
  `any` or `not` instead);
* an unknown predicate function inside `cfg`, such as `cfg(foo(x))`;
* an `extern` block with an ABI other than `"C"`.

Lunar has no `derive`: traits such as `Eq`, `Clone` or `Display` are
implemented with an explicit `impl`.

An unknown `cfg` name or value such as `#[cfg(linux)]` is not an error: it
is false, so the item is compiled out, and the compiler warns about it with
`lint::unknown_cfg`. Use `#[cfg(target_os = "linux")]`.

## Erroneous example

```lunar
#[derive(Eq)]
struct Point { x: i32, y: i32 }

fn main() {
    let a = Point { x: 1, y: 2 };
    println("{a == Point { x: 1, y: 2 }}");
}
```

## Fixed

`Eq` is implemented by hand:

```lunar
struct Point { x: i32, y: i32 }

impl Eq for Point {
    fn eq(&self, other: &Point): bool {
        return self.x == other.x && self.y == other.y;
    }
}

fn main() {
    let a = Point { x: 1, y: 2 };
    println("{a == Point { x: 1, y: 2 }}");
}
```
