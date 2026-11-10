# type::grow_outside_param: a `+T` type is used somewhere other than a parameter

`+T` marks an in-out parameter that the function only grows (`x: &mut +T`,
or a `&mut +self` receiver), which tells callers their views into the
argument survive the call. It has no meaning anywhere else: a `let`
binding, a struct field, a return type, or a type nested inside another
type such as `[+[i32; ..+]; 2]` cannot be `+`.

## Erroneous example

```lunar
fn main() {
    let mut log: +[i32; ..+] = [];
    log.push(1);
}
```

## Fixed

Drop the `+`:

```lunar
fn main() {
    let mut log: [i32; ..+] = [];
    log.push(1);
}
```
