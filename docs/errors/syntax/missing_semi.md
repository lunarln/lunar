# syntax::missing_semi: a statement is missing its `;`

Inside a block, every `let` statement and every expression statement must
end with a semicolon. Only two things may go without one:

* the last expression of a block, which becomes the block's value (a tail
  expression);
* block-like expressions used as statements: `if`, `match`, loops and
  blocks.

The error points at the place where the `;` should go, right after the end
of the previous statement or expression.

A missing `;` after an item that needs one (`use`, `const`, `static`,
`type`, `struct S;`, `mod m;`) is reported as `syntax::unexpected_token`
instead, pointing at the token that follows.

## Erroneous example

```lunar
fn main() {
    let a = 1
    let b = 2;
    println("{a} {b}");
}
```

## Fixed

The `let` statement ends with `;`:

```lunar
fn main() {
    let a = 1;
    let b = 2;
    println("{a} {b}");
}
```
