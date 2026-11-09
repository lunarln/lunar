# type::annotations_needed: the compiler cannot infer a type

Type inference inside a function body could not determine some type, and
there is no annotation to fix it. A typical case is an empty array literal
or a generic call whose type parameters are never constrained by later uses.
Only the first such spot in a function is reported.

The same code covers the related cases where a type must be written out:

* `let x;` without an initializer: every `let` must have a value;
* `_` or an inferred length `[T; _]` in an item signature (function
  parameters and return types, struct fields, constants): inference only
  works inside function bodies, so signatures must be complete.

Integer and float literals never need annotations: when nothing else
constrains them they default to `i32` and `f64`.

## Erroneous example

```lunar
fn main() {
    let items = [];
    println("{items.len()}");
}
```

## Fixed

Annotate the binding so the item and array kinds are known:

```lunar
fn main() {
    let items: [i32; ..+] = [];
    println("{items.len()}");
}
```
