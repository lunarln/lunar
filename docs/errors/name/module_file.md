# name::module_file: the file for a `mod name;` declaration was not found

A `mod name;` declaration (without a body) loads the module from another
file, and neither candidate file exists. For a crate root or a `mod.ln`
file, the compiler looks for `name.ln` and then `name/mod.ln` in the same
directory. For any other file `parent.ln`, it looks in the `parent/`
directory next to it. The help lists the exact paths that were tried.

Create one of those files, fix the module name, or write the module inline
as `mod name { ... }`.

## Erroneous example

```lunar
mod helpers;

fn main() {
    println("{helpers::double(21)}");
}
```

## Fixed

The module is written inline (alternatively, create `helpers.ln` next to the
main file):

```lunar
mod helpers {
    pub fn double(x: i32): i32 {
        return x * 2;
    }
}

fn main() {
    println("{helpers::double(21)}");
}
```
