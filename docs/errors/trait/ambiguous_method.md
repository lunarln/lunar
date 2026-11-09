# trait::ambiguous_method: a method call matches methods of several traits

A method call `value.name(...)` found no inherent method called `name`, and
more than one trait that the value's type implements (or is bounded by, for
a generic parameter) provides a method with that name.

Lunar resolves method calls statically and does not guess which trait was
meant, because picking one silently could call the wrong code. The compiler
lists the candidate traits.

Call the method through the trait instead: `Trait::name(value, ...)` names
exactly one method.

## Erroneous example

```lunar
trait Walk {
    fn go(self): i32;
}

trait Swim {
    fn go(self): i32;
}

struct Duck;

impl Walk for Duck {
    fn go(self): i32 {
        return 1;
    }
}

impl Swim for Duck {
    fn go(self): i32 {
        return 2;
    }
}

fn main() {
    let d = Duck;
    println("{d.go()}");
}
```

## Fixed

Say which trait's method to call:

```lunar
trait Walk {
    fn go(self): i32;
}

trait Swim {
    fn go(self): i32;
}

struct Duck;

impl Walk for Duck {
    fn go(self): i32 {
        return 1;
    }
}

impl Swim for Duck {
    fn go(self): i32 {
        return 2;
    }
}

fn main() {
    let d = Duck;
    println("{Walk::go(d)} {Swim::go(d)}");
}
```
