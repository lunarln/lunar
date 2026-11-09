# syntax::bad_format: a format string is malformed

This code is reserved for malformed format strings, but the compiler does
not currently emit it.

Problems inside a string literal passed to a `FmtArgs` parameter (such as
`println`), like an unclosed `{`, a stray `}` or an invalid expression
between braces, are reported as `type::interpolation`. See
`lunar explain type::interpolation` for details and examples.
