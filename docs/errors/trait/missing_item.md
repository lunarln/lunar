# trait::missing_item: an impl does not define a required trait method

An `impl Trait for T` block leaves out a method that the trait declares
without a default body.

Code that uses a type through a trait, for example a generic function with a
`T: Trait` bound, may call any of the trait's methods. Every implementation
must therefore provide each method that has no default. Methods with a
default body in the trait may be left out; the default is used.

The error points at the type in the impl header and names the missing
method; a secondary label shows where the trait declares it.

## Erroneous example

```lunar
trait Speak {
    fn speak(&self): str;
    fn times(&self): i32 {
        return 1;
    }
}

struct Dog;

impl Speak for Dog {}

fn main() {}
```

## Fixed

Define `speak` in the impl. `times` has a default, so it can still be left
out:

```lunar
trait Speak {
    fn speak(&self): str;
    fn times(&self): i32 {
        return 1;
    }
}

struct Dog;

impl Speak for Dog {
    fn speak(&self): str {
        return "woof";
    }
}

fn main() {
    let d = Dog;
    println("{d.speak()} x{d.times()}");
}
```
