# syntax::unterminated: a string or character literal is not closed

A string literal was opened with `"` but never closed, or a character
literal starting with `'` has no closing `'` on the same line. A character
literal that is closed but empty (`''`) or holds several characters
(`'ab'`) is `syntax::bad_char`.

An unclosed string runs to the end of the file, so this error is usually
followed by an `unexpected_token` error at "end of file". Fixing the literal
fixes both. A `"` inside a string must be escaped as `\"`, and a newline
inside a character literal is not allowed (write `'\n'`).

Block comments are different: an unclosed `/*` comment silently extends to
the end of the file.

## Erroneous example

```lunar
fn main() {
    let mut s = String::new();
    s.push('!);
    println("{s}");
}
```

## Fixed

The character literal is closed:

```lunar
fn main() {
    let mut s = String::new();
    s.push('!');
    println("{s}");
}
```
