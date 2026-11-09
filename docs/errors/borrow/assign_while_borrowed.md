# borrow::assign_while_borrowed: a value is replaced while a view into it is still in use

A new value was assigned to a variable (or to a field of it) that owns
data, while a slice or `str` pointing into the old value was still going to
be used afterwards.

Assigning to an owner frees the value it held before. Any view into that
old value would then point at freed memory, so the assignment is rejected
while such a view is live. This applies to any place whose type owns data
(an owning array, a `String`, or a type containing one), including
assignments to a field.

The check is based on liveness: a view that is not used after the
assignment does not block it. Assigning to Dull fields, or writing through
a mutable slice, does not free anything and is not affected.

## Erroneous example

```lunar
fn main() {
    let mut line: String = String::from("hello");
    let word: str = line[0..2];
    line = String::from("bye");
    println("{word} {line}");
}
```

## Fixed

Finish using the view before replacing the value:

```lunar
fn main() {
    let mut line: String = String::from("hello");
    let word: str = line[0..2];
    println("{word}");
    line = String::from("bye");
    println("{line}");
}
```
