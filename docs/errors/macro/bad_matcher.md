# macro::bad_matcher: a macro's rule is malformed

A macro definition's matcher or body is not well formed:

* a fragment kind that does not exist (`$x:expression`); the kinds are
  `ident`, `path`, `expr`, `ty`, `pat`, `block`, `stmt`, `item`, `lit`,
  `vis`, `meta`, `tt` and `generics`;
* `$name` in a matcher without a kind (`$x` instead of `$x:expr`);
* a repetition `$( ... )` not followed by `*`, `+` or `?` (with an optional
  separator before it, as in `$( ... ),*`);
* a fragment followed by a token that could continue it, so the matcher
  could not tell where the fragment ends: an `expr` may only be followed by
  `=>`, `,` or `;`, and a `ty` or `path` by one of `=>`, `,`, `=`, `|`, `;`,
  `:`, `>`, `[`, `{` or `as`;
* a body that uses `$name` the matcher does not bind.

## Erroneous example

```lunar
macro double($x:expression) { $x * 2 }

fn main() {
    println("{double!(21)}");
}
```

## Fixed

```lunar
macro double($x:expr) { $x * 2 }

fn main() {
    println("{double!(21)}");
}
```
