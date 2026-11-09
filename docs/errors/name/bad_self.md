# name::bad_self: `self` is used as a parameter outside the first position of a method

A `self` parameter makes a function a method: it receives the value the
method is called on. It is only allowed as the first parameter of a function
inside an `impl` or `trait` block. This code is reported when `self` appears
in a later position, or in a free function that is not inside an `impl` or
`trait`.

A function inside an `impl` without `self` is an associated function, called
as `Type::name(...)`.

## Erroneous example

```lunar
struct Counter { n: i32 }

impl Counter {
    fn add(amount: i32, &mut self) {
        self.n += amount;
    }
}

fn main() {
    let mut c = Counter { n: 0 };
    c.add(2);
    println("{c.n}");
}
```

## Fixed

`self` comes first:

```lunar
struct Counter { n: i32 }

impl Counter {
    fn add(&mut self, amount: i32) {
        self.n += amount;
    }
}

fn main() {
    let mut c = Counter { n: 0 };
    c.add(2);
    println("{c.n}");
}
```
