# name::unresolved: a name or path does not refer to anything in scope

A name used in the program could not be found. Lunar looks a single name up
in the current scope: local variables, items of the current module, its
`use` imports and the prelude. A path such as `std::io::println` is looked
up one segment at a time, and the error names the first segment that does
not exist.

This code is reported for:

* a misspelled or undeclared variable, function, type or module (the help
  suggests a similar item name when one exists);
* a `use` that imports something that does not exist;
* `super` in a crater root, which has no parent module;
* `Self` outside an `impl` or `trait`;
* a path that continues past something that has no such member, such as a
  missing associated function or struct path.

Types and values live in separate namespaces. A named-field struct, a trait
or a module used where a value is expected is not found in the value
namespace, so it is also reported as unresolved.

## Erroneous example

```lunar
fn main() {
    let count = 3;
    println("{coutn}");
}
```

## Fixed

The variable name is spelled correctly:

```lunar
fn main() {
    let count = 3;
    println("{count}");
}
```
