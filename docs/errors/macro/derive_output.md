# macro::derive_output: a derive expanded to something other than items

A derive macro (`#[derive_macro]`) runs on the struct or enum it is
attached to and may only *add* items next to it, usually an `impl`. Its
expansion is parsed as a list of items, and this error means it was
something else, such as an expression or a statement.

The struct or enum itself is never changed by a derive.

## Erroneous example

```lunar
trait Hello {
    fn hello(&self): str;
}

#[derive_macro]
macro Hello {
    (struct $n:ident { $($f:ident: $t:ty),* $(,)? }) => { "hello" },
}

#[derive(Hello)]
struct Point {
    x: i32,
}

fn main() {}
```

## Fixed

```lunar
trait Hello {
    fn hello(&self): str;
}

#[derive_macro]
macro Hello {
    (struct $n:ident { $($f:ident: $t:ty),* $(,)? }) => {
        impl Hello for $n {
            fn hello(&self): str {
                return "hello";
            }
        }
    },
}

#[derive(Hello)]
struct Point {
    x: i32,
}

fn main() {
    let p = Point { x: 1 };
    println("{p.hello()} {p.x}");
}
```
