# macro::error: a macro reported an error itself

A macro can stop with an error of its own by expanding to the metavariable
expression `${error("message")}`, usually in a last rule that catches input
the macro does not support. The message is the macro author's explanation;
this page only says where it comes from.

The derives in `core` use it: `#[derive(Display)]` only applies to enums
whose variants have no fields, and `#[derive(Default)]` on an enum needs a
variant marked `#[default]`.

## Erroneous example

```lunar
#[derive(Display)]
struct Point {
    x: i32,
    y: i32,
}

fn main() {}
```

## Fixed

How a struct is shown is a choice, so `Display` is written by hand:

```lunar
struct Point {
    x: i32,
    y: i32,
}

impl Display for Point {
    fn fmt(&self, out: &mut String) {
        out.write("({self.x}, {self.y})");
    }
}

fn main() {
    let p = Point { x: 1, y: 2 };
    println("{p}");
}
```
