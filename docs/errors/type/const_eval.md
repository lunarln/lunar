# type::const_eval: a constant cannot be evaluated at compile time

The values of `const` and `static` items, and the lengths of fixed array
types such as `[T; N]`, are computed by the compiler. Only a small set of
expressions can be evaluated there: literals, paths to other constants,
parentheses, `as` casts, unary and binary operators on numbers and
booleans, and array, repeat and tuple literals built from those. This error
is reported when:

* the expression uses something that is not supported in constants, such
  as a function call, or a path that does not name a constant;
* the evaluation fails, for example division or remainder by zero;
* an operator is applied to operands it does not support in a constant;
* a `static` has a type that contains an owning array or `String`. Statics
  live for the whole program and are not on any data stack, so they cannot
  own one.

## Erroneous example

```lunar
fn size(): usize {
    return 4;
}

const N: usize = size();

fn main() {
    println("{N}");
}
```

## Fixed

The constant is now written as an expression the compiler can evaluate:

```lunar
const N: usize = 2 * 2;

fn main() {
    println("{N}");
}
```
