# syntax::bad_format: a format string is malformed

A string literal passed to a `FmtArgs` parameter (as in `println`, `print`,
`panic` or `String::write`) is a format string: each `{expr}` is replaced by
`expr` formatted with `Display`. This error is reported when such a string:

* has a `{` without a matching `}`;
* has a `}` that does not close a `{`;
* has an empty `{}`, with no expression inside;
* has a `{...}` whose contents are not a valid expression.

To write a literal brace, double it: `{{` and `}}`. String literals that are
not passed to a `FmtArgs` parameter are plain `str` values and are never
interpolated, so braces in them need no escaping.

A well-formed hole whose value cannot be formatted, because its type does
not implement `Display`, is a different error: `type::unsatisfied_bound`.

## Erroneous example

```lunar
fn main() {
    let x = 3;
    println("set: {x");
}
```

## Fixed

The placeholder is now closed, and the literal braces around it are doubled:

```lunar
fn main() {
    let x = 3;
    println("set: {{{x}}}");
}
```
