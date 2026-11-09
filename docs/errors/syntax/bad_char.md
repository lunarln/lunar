# syntax::bad_char: a character literal does not hold exactly one character

A character literal `'...'` is a single `char`: one Unicode scalar value.
This error is reported when the literal is empty (`''`) or holds more than
one character, whether written directly (`'ab'`) or with escapes
(`'\u{41}\u{42}'`).

For text, use a string literal (`"ab"`), which is a `str`. For a space write
`' '`, and for the NUL character `'\0'`.

An invalid escape inside a character literal, such as `'\q'`, is
`syntax::bad_escape`, and a `'` that is never closed on its line is
`syntax::unterminated`.

## Erroneous example

```lunar
fn main() {
    let mut s = String::new();
    s.push('ok');
    println("{s}");
}
```

## Fixed

Text is appended as a string, one character as a `char`:

```lunar
fn main() {
    let mut s = String::new();
    s.push_str("ok");
    s.push('!');
    println("{s}");
}
```
