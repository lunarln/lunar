# type::from_without_views: a `from` or `via` clause names something that holds no views

A function's `from` clause says which parameters its result may point into,
and its `via` clause which parameters' views it may point where (see
`borrow::escapes_via_return`). One was given where nothing can point:

* the return type holds no views (a number, a `String`, a struct of
  owners), so the result points into no parameter and needs no clause;
* `from` names a parameter that takes its argument by value and holds no
  views: a Dull value (`n: usize`) is a copy the function owns, and an
  owner (`s: String`) is freed when the function returns, so a returned
  view cannot point into either. A borrowed parameter (`s: &String`) or
  one that holds views (`s: str`) can be named.
* `via` names a parameter that holds no views, whatever its mode: a
  `n: usize`, or a borrowed `s: &String`, whose own data the result could
  point into, but which points nowhere itself. Name a borrowed parameter in
  `from` if the result points into its data. A parameter holding a `str`,
  a slice or a struct with one can be named in `via`.

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
