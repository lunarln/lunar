# borrow::views_itself: a `dyn mut` stored inside the value it views

A `dyn mut` that views a variable (or a part of it) is stored into that same
variable.

A method called through a `dyn mut` may shrink or replace what it views,
which ends every view into it. The borrow checker sees that where the
method is called on a `dyn mut` it knows to view the variable. Once the
`dyn mut` is inside the variable itself, code that only has the variable
(a function it is lent to) could call through it and shrink the variable
while holding views into it, and nothing would tell.

## Erroneous example

```lunar
trait Reset {
    fn reset(&mut self);
}

impl Reset for String {
    fn reset(&mut self) {
        self.clear();
    }
}

struct Doc {
    text: String,
    target: dyn mut Reset,
}

fn rewire(d: &mut Doc) {
    d.target = d.text as dyn mut Reset;
}

fn main() {}
```

## Fixed

Keep the `dyn mut` next to the value, not inside it:

```lunar
trait Reset {
    fn reset(&mut self);
}

impl Reset for String {
    fn reset(&mut self) {
        self.clear();
    }
}

struct Doc {
    text: String,
}

fn clear_it(target: dyn mut Reset) {
    target.reset();
}

fn main() {
    let mut d = Doc { text: String::from("draft") };
    clear_it(d.text as dyn mut Reset);
    println("{d.text.len()}");
}
```
