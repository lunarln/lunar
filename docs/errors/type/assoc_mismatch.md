# type::assoc_mismatch: an associated type is not the type a constraint requires

A constraint on an associated type says what it must be:

* a bound `I: Iterator::<Item = u64>`: the argument given for `I` must be an
  iterator of `u64`;
* a return type `impl Iterator::<Item = u8>`: what the function returns must
  be an iterator of `u8`;
* a trait object `x as dyn Iterator::<Item = u8>`: `x` must iterate `u8`s;
* a supertrait `trait Int: Add::<Output = Self>`: every type implementing
  `Int` must add up to itself.

The type the associated type is for the type at hand (its impl's
`type Item = ...;`) is another one. The message says which projection was
checked (`<Chars as Iterator>::Item`), what it is, and what is required.

## Erroneous example

```lunar
fn total::<I: Iterator::<Item = u64>>(it: I): u64 {
    let mut t: u64 = 0;
    for x in it {
        t += x;
    }
    return t;
}

fn main() {
    let n = total("abc".chars());
    println("{n}");
}
```

## Fixed

Give an iterator of `u64`: here, each character converted.

```lunar
fn total::<I: Iterator::<Item = u64>>(it: I): u64 {
    let mut t: u64 = 0;
    for x in it {
        t += x;
    }
    return t;
}

fn main() {
    let n = total("abc".chars().map(|c| c as u64));
    println("{n}");
}
```
