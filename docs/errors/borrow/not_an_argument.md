# borrow::not_an_argument: `&` or `&mut` is used outside a call argument

`&v` and `&mut v` only say how an argument is lent to a function. They do
not make a value: Lunar has no references to store in variables, fields or
arrays. Borrowing is a parameter mode, and the call is where it is written.

To keep a view of part of an array or string, use a slice (`a[1..3]`, of
type `[T; &]`) or `str`. To share a value, pass it to a function that takes
`&T`.

## Erroneous example

```lunar
fn main() {
    let name = String::from("lunar");
    let r = &name;
    println("{r}");
}
```

## Fixed

```lunar
fn main() {
    let name = String::from("lunar");
    let r = name.as_str();
    println("{r}");
}
```
