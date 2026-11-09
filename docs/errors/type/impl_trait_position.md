# type::impl_trait_position: `impl Trait` outside a function's return type

`impl Trait` names "some type with these bounds, which the function body
decides". It is allowed only as (part of) the return type of a function
with a body that is not a trait method or a method of a trait's impl.

To accept any type with some bounds, use a generic parameter instead; Lunar
has no `impl Trait` in parameter position. A trait method cannot return
`impl Trait`, because each implementation would return a different type.

## Erroneous example

```lunar
fn apply(f: impl Fn(i32): i32): i32 {
    return f(1);
}

fn main() {
    println("{apply(|x| x + 1)}");
}
```

## Fixed

```lunar
fn apply::<F: Fn(i32): i32>(f: F): i32 {
    return f(1);
}

fn main() {
    println("{apply(|x| x + 1)}");
}
```
