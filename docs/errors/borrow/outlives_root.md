# borrow::outlives_root: a view would outlive the value it points into

A slice or `str` was stored in a variable that lives longer than the value
the view points into.

Owners are freed when they go out of scope. A view held by a variable from
an outer scope would still be reachable after that, pointing at freed
memory. The compiler compares scopes when a view is bound with `let` or
assigned to a variable, and rejects the store if the viewed value is
declared in a more deeply nested scope than the variable receiving the
view.

The same error is reported when a view points into a temporary, such as the
result of a function call that returns an owning value: the temporary is
freed at the end of the statement, so `let v = make()[0..2];` would leave
`v` dangling.

## Erroneous example

```lunar
fn main() {
    let mut longest: str = "";
    {
        let line: String = String::from("a temporary line");
        longest = line.as_str();
    }
    println("{longest}");
}
```

## Fixed

Declare the viewed value in a scope at least as long-lived as the view
(for a temporary, bind it to a variable first):

```lunar
fn main() {
    let line: String = String::from("a temporary line");
    let mut longest: str = "";
    {
        longest = line.as_str();
    }
    println("{longest}");
}
```
