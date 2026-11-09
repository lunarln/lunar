# type::not_callable: something that is not a function is called, or a function cannot become a pointer

A call expression `f(...)` was written where `f` is not a function: a local
variable, constant or static that is not a function pointer, or an enum
variant that is not a tuple variant. Unit variants are written without
parentheses (`Dir::North`), and variants with named fields use braces
(`Shape::Rect { w: 1.0, h: 2.0 }`).

The same code is reported when a function is used as a value (a function
pointer) but cannot be one:

* functions with in-out (`x: &mut T`) parameters, because a function pointer
  type `fn(A): R` has no way to express in-out arguments;
* functions with parameters that take ownership of an owned value (such as
  `s: String`), because calls through a function pointer always borrow
  their arguments. Borrow the parameter (`s: &String`) instead; parameters
  of Dull types may still be taken by value;
* compiler intrinsics, which have no address;
* trait methods, because trait calls are resolved statically and there are no
  trait objects. Call the method directly, or wrap it in a plain function.

## Erroneous example

```lunar
fn main() {
    let limit: i32 = 10;
    let n = limit(3);
    println("{n}");
}
```

## Fixed

Only call functions; here the intent was a plain function computing a
value:

```lunar
fn limit(n: i32): i32 {
    return n * 10;
}

fn main() {
    let n = limit(3);
    println("{n}");
}
```
