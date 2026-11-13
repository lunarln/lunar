# syntax::missing_value: a constant or type alias without its value

`const N: usize;` and `type Item;` (without `= ...`) declare an associated
constant and an associated type: in a trait, they say that each impl of the
trait gives one. Anywhere else, a constant needs its value and a type alias
its type:

* a constant in a module, or an associated constant in an impl:
  `const N: usize = 4;`;
* a type alias in a module (`type Bytes = [u8; &];`), or an associated
  type in an impl (`type Item = u8;`).

## Erroneous example

```lunar
trait Shape {
    const SIDES: u32;
}

struct Square;

impl Shape for Square {
    const SIDES: u32;
}

fn main() {}
```

## Fixed

The impl gives the constant its value:

```lunar
trait Shape {
    const SIDES: u32;
}

struct Square;

impl Shape for Square {
    const SIDES: u32 = 4;
}

fn main() {
    println("{Square::SIDES}");
}
```
