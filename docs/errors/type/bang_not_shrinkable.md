# type::bang_not_shrinkable: `!` is applied to a type that cannot shrink

A parameter was declared `!T`, but nothing inside `T` can ever shrink. `!`
only makes sense when `T` is `[U; -..+]`, `String`, or a struct, enum or
tuple that contains one at any depth. Arrays of the other kinds (`[U; N]`,
`[U]`, `[U; ..+]`, slices) never shrink, so slices into them are never
invalidated and `!` would only restrict callers for no reason.

A generic parameter `T` is accepted, since it might be instantiated with a
shrinkable type, unless it is bounded by `T: Dull`: Dull types contain no
arrays, so they can never shrink.

## Erroneous example

```lunar
struct Log {
    lines: [i32; ..+],
}

fn add(mut log: !Log, line: i32) {
    log.lines.push(line);
}

fn main() {
    let mut log = Log { lines: [] };
    add(log, 7);
}
```

## Fixed

`add` only grows the log, which never invalidates views, so a plain `mut`
parameter is enough:

```lunar
struct Log {
    lines: [i32; ..+],
}

fn add(mut log: Log, line: i32) {
    log.lines.push(line);
}

fn main() {
    let mut log = Log { lines: [] };
    add(log, 7);
}
```
