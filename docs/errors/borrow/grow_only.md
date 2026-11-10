# borrow::grow_only: a `&mut +` parameter is shrunk or replaced

A parameter declared `x: &mut +T` (or a `&mut +self` receiver) promises its
callers that the function only grows the argument, so their slices and
`str` views into it stay valid across the call. The function broke that
promise: it shrinks the value (`pop`, `truncate`, `clear`, `remove`),
replaces an owner inside it (`x.items = [];` frees the old array), lends it
to a plain `&mut` parameter (which may do either), or calls a closure that
changes it.

Inside a `+` function, the parameter may be read, changed in its Dull parts
(`x.count += 1`, `x.items[i] = v`), grown with `+` methods such as `push`,
`extend` or `push_str`, and passed on to other `&mut +` parameters.

## Erroneous example

```lunar
struct Log {
    lines: [i32; -..+],
}

fn add(log: &mut +Log, line: i32) {
    log.lines.push(line);
    if log.lines.len() > 100 {
        log.lines.clear();
    }
}

fn main() {
    let mut log = Log { lines: [] };
    add(&mut log, 7);
    println("{log.lines.len()}");
}
```

## Fixed

The function may clear the log, so drop the `+`. Callers can no longer keep
views into the log across a call to `add`:

```lunar
struct Log {
    lines: [i32; -..+],
}

fn add(log: &mut Log, line: i32) {
    log.lines.push(line);
    if log.lines.len() > 100 {
        log.lines.clear();
    }
}

fn main() {
    let mut log = Log { lines: [] };
    add(&mut log, 7);
    println("{log.lines.len()}");
}
```
