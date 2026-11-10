# borrow::shrink_while_borrowed: a value may be shrunk while a view into it is still in use

A value was lent in-out where it may be shrunk or replaced, while a slice or
`str` pointing into it was still going to be used afterwards. That happens
when it is passed to a `&mut` parameter, has a `&mut self` method called on
it (such as `pop`, `truncate` or `clear`), or is changed by a closure that
borrowed it.

Growing an owning array never moves its elements, so views taken from it stay
valid across `push`. Shrinking is different: once elements are removed, a
view that covered them would point at memory that no longer holds live
values, and replacing an owner (`self.items = [];`) frees the old array. A
plain `&mut` parameter may do either, so the compiler ends every view into
the value at the call, and rejects the call if one of those views is used
later. A parameter declared `&mut +T` (or `&mut +self`) only grows its
argument, so calls to it keep views valid: `push`, `extend`, `push_str` and
`write` are declared that way.

The check is based on liveness, not scope. A view that is never used again
after the call does not block it, even if its variable is still in scope.
Views are tracked per variable: a view into one field of a struct is ended
by lending the whole struct.

The same error is reported when one argument of a call may shrink a value
that another argument of the same call points into.

A view returned by a function points into the arguments that the
function's body returns views of (and, through a function pointer or a
trait method on a generic receiver, into all of them). When the view comes
from what a function's body returns, the error also points at that
argument: "the value returned by `f` points into `x`".

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
