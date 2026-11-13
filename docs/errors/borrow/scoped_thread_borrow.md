# borrow::scoped_thread_borrow: a scoped thread borrows a variable of the scope's closure

A thread started with `Scope::spawn` borrows (or holds a view of) a variable
declared inside the closure given to `thread::scope`.

`thread::scope(|s| ...)` runs the closure, and *then* waits for every thread
the closure started. The closure's own variables are freed when it returns,
which is before the threads are waited for, so a thread borrowing one could
still be reading it after it is gone. Threads may borrow what the closure
itself borrows from outside: that outlives the whole `scope` call.

## Erroneous example

```lunar
use std::thread;

fn main() {
    thread::scope(|s| {
        let words = String::from("one two three");
        s.spawn(|| words.len());
    });
}
```

## Fixed

Declare the variable before `thread::scope`, so it outlives the threads:

```lunar
use std::thread;

fn main() {
    let words = String::from("one two three");
    thread::scope(|s| {
        s.spawn(|| words.len());
    });
}
```

Or move it into the thread with a `move` closure, if only that thread needs
it.
