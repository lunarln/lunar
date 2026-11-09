# type::bad_operand: an operator is applied to a type that does not support it

An operator was used on a value of a type it is not defined for.

On primitive types the rules are fixed: arithmetic (`+ - * / %`) needs
numbers; `& | ^` need integers or `bool`; `<< >>` need integers; ordering
comparisons (`< <= > >=`) are not defined for `bool`. A negative literal
cannot have an unsigned type, and `*x` needs a raw pointer.

On other types, operators are calls to the traits in `core::ops` and
`core::cmp`: `a + b` needs `Add`, `a == b` needs `Eq`, `a < b` needs `Ord`,
and so on. If the type does not implement the trait, the operator is
rejected. For a generic `T`, add the trait to `T`'s bounds.

The same code covers misused ranges: a range with a missing end (`a..`) is
only allowed when slicing, and an inclusive range needs an end (`a..=b`) and
is only supported in `for` loops and slices.

## Erroneous example

```lunar
struct Money {
    cents: i64,
}

fn main() {
    let a = Money { cents: 150 };
    let b = Money { cents: 275 };
    let total = a + b;
    println("{total.cents}");
}
```

## Fixed

Implement `core::ops::Add` for the type:

```lunar
struct Money {
    cents: i64,
}

impl core::ops::Add for Money {
    fn add(self, rhs: Money): Money {
        return Money { cents: self.cents + rhs.cents };
    }
}

fn main() {
    let a = Money { cents: 150 };
    let b = Money { cents: 275 };
    let total = a + b;
    println("{total.cents}");
}
```
