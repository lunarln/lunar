# macro::no_match: no rule of the macro matches the invocation

A macro's rules are tried in order, and the first whose matcher accepts all
of the invocation's tokens is used. This error means none did: the
invocation has too many or too few arguments, a separator the matcher does
not expect (`;` instead of `,`), or a fragment of the wrong kind (a type
where an expression is expected).

Compare the invocation with the macro's matchers. A matcher's literal
tokens must appear exactly, and each `$x:kind` must be followed by a
complete fragment of that kind.

## Erroneous example

```lunar
macro pair($a:expr, $b:expr) { ($a, $b) }

fn main() {
    let (x, y) = pair!(1);
    println("{x} {y}");
}
```

## Fixed

```lunar
macro pair($a:expr, $b:expr) { ($a, $b) }

fn main() {
    let (x, y) = pair!(1, 2);
    println("{x} {y}");
}
```
