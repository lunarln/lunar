# macro::not_supported: a macro feature that is not implemented yet

The macro is valid, but uses something the compiler cannot expand yet.
Currently that is defining items (functions, structs, ...) in a macro that
is expanded in expression or statement position, that is, inside a
function body.

Define the items outside the macro, or invoke a macro in item position
(outside any function) to define them.

## Erroneous example

```lunar
macro helper() {
    fn twice(x: i32): i32 {
        return x * 2;
    }
    twice(21)
}

fn main() {
    println("{helper!()}");
}
```

## Fixed

```lunar
fn twice(x: i32): i32 {
    return x * 2;
}

macro helper() { twice(21) }

fn main() {
    println("{helper!()}");
}
```
