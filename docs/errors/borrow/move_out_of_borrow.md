# borrow::move_out_of_borrow: an owned value is moved out of something this code does not own

Code tried to move an owning value (an owning array, a `String`, or a type
containing one) out of a place it only borrows.

A parameter declared `x: &T` (or `x: &mut T`, or a `&self` / `&mut self`
receiver) is borrowed: the caller keeps ownership and frees the value after
the call returns. If the callee could move the parameter, or an owning field
of it, into a new owner, both would end up freeing the same data. The same
applies to an owned value reached through an array element, a `static`, or a
raw pointer: none of these places can be left empty, so their contents
cannot be moved out.

The error is reported for:

* moving a borrowed parameter, or a field of one (by binding, assigning,
  returning, passing it to a parameter that takes ownership, or matching it
  by value);
* moving an owned value out of an array element;
* moving an owned value out of a `static` or through a raw pointer.

Reading Dull fields and taking views is fine; only moves of owned data are
rejected.

## Erroneous example

```lunar
struct Inbox {
    messages: [i32; ..+],
}

fn take_all(inbox: &Inbox): [i32; ..+] {
    return inbox.messages;
}

fn main() {
    let inbox = Inbox { messages: [1, 2] };
    let all = take_all(inbox);
    println("{all.len()}");
}
```

## Fixed

Take ownership of the parameter (`inbox: Inbox`), so the function may move
its fields; the caller's `inbox` is moved into the call. If the caller still
needs its value, keep `&Inbox` and return `inbox.messages.clone()` instead:

```lunar
struct Inbox {
    messages: [i32; ..+],
}

fn take_all(inbox: Inbox): [i32; ..+] {
    return inbox.messages;
}

fn main() {
    let inbox = Inbox { messages: [1, 2] };
    let all = take_all(inbox);
    println("{all.len()}");
}
```
