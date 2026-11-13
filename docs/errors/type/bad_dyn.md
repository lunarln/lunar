# type::bad_dyn: a `dyn` type without exactly one trait, with bounds other than `Send` and `Sync`, or without its associated types

A `dyn` type names no trait to call methods through, more than one, an
auto trait other than `Send` and `Sync`, or leaves out what the trait's
associated types are.

`dyn Trait` views a value through the vtable of one trait (and that trait's
supertraits). After the trait it may say `+ Send` and `+ Sync`: that the
value viewed is Send or Sync, checked where the value is first viewed as
`dyn`. Nothing else may follow: a `dyn` is always Light, never Dull or
Detached (it is a view), whatever it views.

To call the methods of two traits through one `dyn`, make a trait with
both as supertraits.

The trait's associated types (and its supertraits') must all be given:
`dyn Iterator::<Item = u8>`, not `dyn Iterator`. A call through the vtable
cannot find out the type of the value viewed, so the `dyn` type says what
its items are.

## Erroneous example

```lunar
trait Draw {
    fn draw(&self);
}

trait Name {
    fn name(&self): str;
}

fn show(item: dyn Draw + Name) {
    item.draw();
}

fn main() {}
```

## Fixed

```lunar
trait Draw {
    fn draw(&self);
}

trait Name {
    fn name(&self): str;
}

trait Widget: Draw + Name {}

fn show(item: dyn Widget) {
    item.draw();
    println("{item.name()}");
}

fn main() {}
```
