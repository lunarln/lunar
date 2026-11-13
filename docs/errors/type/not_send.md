# type::not_send: a value that cannot move to another thread is sent to one

A value whose type is not `Send` was given where it would move to another
thread: the closure of `thread::spawn` or `Scope::spawn`, its result, a
message on a channel, or any other place bounded by `T: Send`.

`core::marker::Send` is an auto trait: the compiler implements it for a type
when all of its parts are Send. These are not:

* raw pointers (`*T`, `*mut T`): they carry no promise about what they point
  at or who else uses it;
* mutable slices (`[T; &mut]`): they are copied freely, so two threads could
  write through the same one at once;
* shared slices of items that are not `Sync`;
* closures that change a variable they borrow (`|| count += 1`): two threads
  would change it at once;
* types with `impl !Send`, or whose `unsafe impl Send` has bounds that do not
  hold, and anything holding one of these.

The notes under the error follow the type down to the part that is not Send.

A type that holds raw pointers but guards them itself (a handle to memory
nobody else uses, say) can promise Send with `unsafe impl Send for T {}`.
The compiler cannot check that promise, which is why it is `unsafe`.

## Erroneous example

```lunar
use std::thread;

struct Buffer {
    data: *mut u8,
    len: usize,
}

fn main() {
    let b = Buffer { data: core::ptr::null_mut(), len: 0 };
    let h = thread::spawn(move || b.len);
    println("{h.join().unwrap()}");
}
```

## Fixed

If nothing else uses the memory `Buffer` points at, promise that it may move
to another thread:

```lunar
use std::thread;

struct Buffer {
    data: *mut u8,
    len: usize,
}

unsafe impl Send for Buffer {}

fn main() {
    let b = Buffer { data: core::ptr::null_mut(), len: 0 };
    let h = thread::spawn(move || b.len);
    println("{h.join().unwrap()}");
}
```

To change one counter from several threads, share an atomic or a `Mutex`
instead of borrowing a variable in-out from each thread.
