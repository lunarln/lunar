# name::duplicate: the same name is defined more than once

Two definitions in the same scope have the same name, so a use of that name
would be ambiguous. This is reported for:

* two items of the same namespace in one module (functions, constants and
  statics share the value namespace; structs, enums, traits, type aliases
  and modules share the type namespace);
* a `use` that imports a name the module already defines or imports from
  elsewhere (importing the same item twice is allowed);
* two variants of an enum, or two fields of a struct or enum variant, with
  the same name;
* two methods with the same name in one `trait` or in one `impl` block;
* a generic parameter list that repeats a name or declares `Self`.

Local variables are not affected: a `let` may shadow an earlier binding.

## Erroneous example

```lunar
fn area(w: f64, h: f64): f64 {
    return w * h;
}

fn area(r: f64): f64 {
    return 3.14159 * r * r;
}

fn main() {
    println("{area(2.0, 3.0)}");
}
```

## Fixed

Each function has its own name:

```lunar
fn rect_area(w: f64, h: f64): f64 {
    return w * h;
}

fn circle_area(r: f64): f64 {
    return 3.14159 * r * r;
}

fn main() {
    println("{rect_area(2.0, 3.0)} {circle_area(1.0)}");
}
```
