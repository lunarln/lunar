# syntax::bad_escape: a string or character literal contains an invalid escape

A backslash in a string or character literal starts an escape sequence, and
the one found here is not valid. The valid escapes are `\n`, `\t`, `\r`,
`\0`, `\\`, `\"`, `\'`, `\xHH` (two hex digits, at most `\x7F`), `\u{...}`
(a Unicode scalar value in hex) and a backslash at the end of a line (a line
continuation).

The error is reported for an unknown escape such as `\q`, a malformed `\x`
or `\u` escape, a `\u{...}` that is not a valid Unicode scalar value, and a
trailing backslash. Only the first bad escape in a literal is reported.

A character literal whose escapes are valid but produce more (or fewer)
than one character, such as `'\u{41}\u{42}'`, is `syntax::bad_char`.

A Windows path is a frequent cause: in `"C:\Users"`, `\U` is read as an
escape.

## Erroneous example

```lunar
fn main() {
    let path: str = "C:\Users\data";
    println("{path}");
}
```

## Fixed

Each literal backslash is written as `\\`:

```lunar
fn main() {
    let path: str = "C:\\Users\\data";
    println("{path}");
}
```
