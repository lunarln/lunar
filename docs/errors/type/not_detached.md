# type::not_detached: a value holding views must outlive what they point into

A value whose type holds views was given where it may outlive every local
variable: the closure of `thread::spawn` or its result, a message on a
channel, the value of a `Mutex`, `RwLock`, `OnceLock` or `Shared`, the result
of a lock's closure, or anything bounded by `T: Detached`.

`core::marker::Detached` is an auto trait: a type is Detached when it holds
no views, so that nothing in it points into data it does not own. `str`,
slices, closures that borrow, and types holding one of those are not.
Owners (`String`, owning arrays), Dull values and raw pointers are.

A thread started by `thread::spawn` may run after the function that started
it has returned and freed its variables, so the closure must own everything
it uses: make it a `move` closure of owned values. A lock's closure must not
hand out a view of the locked value either: once the lock is let go, another
thread may change or free what the view points into.

To let threads borrow from the function that starts them, use `thread::scope`:
its threads all finish before it returns.

## Erroneous example

```lunar
use std::thread;

fn main() {
    let name = String::from("lunar");
    let view = name.as_str();
    let h = thread::spawn(move || view.len());
    println("{h.join().unwrap()}");
}
```

## Fixed

Move an owned value into the thread:

```lunar
use std::thread;

fn main() {
    let name = String::from("lunar");
    let h = thread::spawn(move || name.len());
    println("{h.join().unwrap()}");
}
```
