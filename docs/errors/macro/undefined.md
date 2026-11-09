# macro::undefined: no macro by that name

An invocation `name!(...)` names something that is not a macro in scope,
or a macro is used the wrong way:

* no macro called `name` is defined in this module, imported with `use`,
  or in the prelude (macros live in their own namespace, so a function or
  type with that name does not count);
* a derive macro (one marked `#[derive_macro]`) is invoked with `!`; derives
  are used as `#[derive(Name)]` on a struct or enum;
* `#[derive(Name)]` names a macro that is not marked `#[derive_macro]`.

Macros are items: a macro defined in another module is imported with
`use`, like a function, and needs `pub` to be used from outside its module.

## Erroneous example

```lunar
macro double($x:expr) { $x * 2 }

fn main() {
    println("{dubble!(21)}");
}
```

## Fixed

```lunar
macro double($x:expr) { $x * 2 }

fn main() {
    println("{double!(21)}");
}
```
