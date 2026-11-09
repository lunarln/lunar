# syntax::missing_semi: a statement or item is missing its `;`

Inside a block, every `let` statement and every expression statement must
end with a semicolon. Only two things may go without one:

* the last expression of a block, which becomes the block's value (a tail
  expression);
* block-like expressions used as statements: `if`, `match`, loops and
  blocks.

The error points at the place where the `;` should go, right after the end
of the previous statement or expression.

Items that do not end in a `{ ... }` block also end with a semicolon:
`use`, `const`, `static`, `type`, unit and tuple structs (`struct S;`,
`struct P(i32);`) and `mod m;`. A missing `;` after one of them is reported
with this code when the next token starts another item (or closes the
enclosing block, or ends the file). Anything else after the item is
`syntax::unexpected_token`.

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
