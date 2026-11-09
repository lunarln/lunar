# borrow::shrink_while_borrowed: a value is shrunk while a view into it is still in use

A value was passed to a `!` parameter, or had a shrinking method such as
`pop`, `truncate` or `clear` called on it, while a slice or `str` pointing
into it was still going to be used afterwards.

Growing an owning array never moves its elements, so views taken from it stay
valid across `push`. Shrinking is different: once elements are removed, a
view that covered them would point at memory that no longer holds live
values. The compiler therefore tracks which views point into which owner and
rejects a shrink if any of those views is used later.

The check is based on liveness, not scope. A view that is never used again
after the shrink does not block it, even if its variable is still in scope.
Views are tracked per variable: a view into one field of a struct blocks
shrinking the whole struct.

The same error is reported when one argument of a call shrinks a value that
another argument of the same call points into.

## Erroneous example

```lunar
fn main() {
    let mut stack: [i64; -..+] = [];
    stack.push(1);
    stack.push(2);
    let top: [i64; &] = stack[1..2];
    stack.pop();
    println("{top[0]}");
}
```

## Fixed

Finish using the view before shrinking, or take a new view afterwards:

```lunar
fn main() {
    let mut stack: [i64; -..+] = [];
    stack.push(1);
    stack.push(2);
    let top: [i64; &] = stack[1..2];
    println("{top[0]}");
    stack.pop();
}
```
