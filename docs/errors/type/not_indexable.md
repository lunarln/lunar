# type::not_indexable: a value that cannot be indexed or sliced is indexed

Indexing `a[i]` and slicing `a[i..j]` work on arrays and slices of every kind.
Other types, such as integers, structs or tuples, cannot be indexed.

`str` and `String` can be sliced with a byte range (`s[0..3]`), but not
indexed by position: UTF-8 characters vary in width, so `s[i]` would have no
clear meaning. Use `s.as_bytes()[i]` to read a byte, or `s.chars()` to walk
the characters.

## Erroneous example

```lunar
fn main() {
    let word = "lunar";
    let first = word[0];
    println("{first}");
}
```

## Fixed

Index the bytes of the string instead:

```lunar
fn main() {
    let word = "lunar";
    let first = word.as_bytes()[0];
    println("{first}");
}
```
