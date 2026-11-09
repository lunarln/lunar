# syntax::unknown_char: the source contains characters that are not part of any token

The lexer found one or more characters that do not start any Lunar token,
such as `$`, `` ` ``, `§` or a non-ASCII symbol outside a string or comment.
A run of adjacent unknown characters is reported as one error.

Identifiers are limited to ASCII letters, digits and `_`, so non-ASCII
letters in names are also reported here. Text in string and character
literals and in comments may contain any Unicode characters.

The unknown characters are skipped, so the parser may report further errors
if removing them leaves invalid code.

## Erroneous example

```lunar
fn main() {
    let total = 10;
    println("{total}");$
}
```

## Fixed

The stray `$` is removed:

```lunar
fn main() {
    let total = 10;
    println("{total}");
}
```
