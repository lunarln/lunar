# borrow::escapes_via_return: a function returns a view of a parameter its signature does not list

A function returned a view (a `str`, a slice, or a value holding one) that
points into a parameter its signature does not say the result points into.

Which arguments a call's result points into is part of the called
function's signature, so that callers can be checked without looking at
its body, and a change to the body cannot break them. The signature says it
with a `from` clause after the return type, or by leaving one out:

* `fn first(a: &String, b: &String): str from a`: the result points into
  `a` only, so the caller may change `b` while it uses the result.
* `from self, key`, `from static` (into no parameter at all, only static
  data such as string literals).
* `via self` (or `from a via t`): only where `self`'s (or `t`'s) views
  point, never into its own data. Returning a view into `self`'s own
  buffer from a `via self` method is this error, since callers keep the
  result across calls that shrink `self`.
* Without a clause, a method that borrows its receiver (`&self`, `&mut
  self`) returns views into `self` only; any other function returns views
  into every parameter it could: each borrowed parameter, and each one
  that holds views itself (a `str`, a struct holding one).

The body is checked against what the signature says: every value it
returns (its tail, its `return`s, through `if`, `match` and blocks) may
point only into the parameters listed. The help lists every parameter the
body's results point into, in `via` those it only points through.

## Erroneous example

```lunar
struct Span {
    lo: usize,
    hi: usize,
}

impl Span {
    fn text(&self, src: str): str {
        return src[self.lo..self.hi];
    }
}

fn main() {
    let s = Span { lo: 0, hi: 2 };
    println("{s.text("hello")}");
}
```

## Fixed

Say that the result points into `src` (and not into `self`):

```lunar
struct Span {
    lo: usize,
    hi: usize,
}

impl Span {
    fn text(&self, src: str): str from src {
        return src[self.lo..self.hi];
    }
}

fn main() {
    let s = Span { lo: 0, hi: 2 };
    println("{s.text("hello")}");
}
```
