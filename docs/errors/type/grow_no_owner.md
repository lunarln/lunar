# type::grow_no_owner: `+` is applied to a type that owns no array

A parameter was declared `&mut +T`, but `T` owns no array or string. `+`
promises that the argument is never shrunk or replaced, so the caller's
views into it survive the call. Only owners can be shrunk or freed: `[U]`,
`[U; ..+]`, `[U; -..+]`, `String`, or a struct, enum, tuple or closure that
contains one. Changing anything else in place (numbers, fixed-size arrays,
slices and `str` views themselves) never invalidates a view, so a plain
`&mut` is already enough.

A generic parameter `T` is accepted, since it might be instantiated with an
owner, unless it is bounded by `T: Dull`.

## Erroneous example

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn nudge(p: &mut +Point) {
    p.x += 1;
}

fn main() {
    let mut p = Point { x: 0, y: 0 };
    nudge(&mut p);
    println("{p.x}");
}
```

## Fixed

Drop the `+`:

```lunar
struct Point {
    x: i32,
    y: i32,
}

fn nudge(p: &mut Point) {
    p.x += 1;
}

fn main() {
    let mut p = Point { x: 0, y: 0 };
    nudge(&mut p);
    println("{p.x}");
}
```
