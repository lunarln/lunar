# type::from_without_views: a `from` clause names something that holds no views

A function's `from` clause says which parameters its result may point into
(see `borrow::escapes_via_return`). It was given where nothing can point:

* the return type holds no views (a number, a `String`, a struct of
  owners), so the result points into no parameter and needs no clause;
* it names a parameter that takes its argument by value and holds no
  views: a Dull value (`n: usize`) is a copy the function owns, and an
  owner (`s: String`) is freed when the function returns, so a returned
  view cannot point into either. A borrowed parameter (`s: &String`) or
  one that holds views (`s: str`) can be named.

## Erroneous example

```lunar
fn tail(s: &String, n: usize): str from s, n {
    return s.as_str()[n..];
}

fn main() {
    let x = String::from("hello");
    println("{tail(&x, 1)}");
}
```

## Fixed

Name only the parameters the result can point into:

```lunar
fn tail(s: &String, n: usize): str from s {
    return s.as_str()[n..];
}

fn main() {
    let x = String::from("hello");
    println("{tail(&x, 1)}");
}
```
