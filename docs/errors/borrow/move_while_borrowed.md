# borrow::move_while_borrowed: a value is moved while a view into it is still in use

An owning value was moved to a new owner while a slice or `str` pointing
into it was still going to be used afterwards.

Views are tied to the variable they were taken from. Once that variable's
value moves, the compiler no longer associates the data with the view: the
new owner could be freed, reassigned or shrunk without the view being
considered, leaving it dangling. A move is therefore treated like any other
invalidating event and rejected while a view into the value is live.

The check is based on liveness: a view that is not used after the move
does not block it. Moving any part of the value that owns data (for
example an owning field of a struct) counts as moving from that variable.

## Erroneous example

```lunar
fn main() {
    let mut log: [i32; ..+] = [1, 2, 3];
    log.push(4);
    let first: [i32; &] = log[0..2];
    let archived = log;
    println("{first[0]} {archived.len()}");
}
```

## Fixed

Finish using the view before moving the value (or take a new view from the
new owner):

```lunar
fn main() {
    let mut log: [i32; ..+] = [1, 2, 3];
    log.push(4);
    let first: [i32; &] = log[0..2];
    println("{first[0]}");
    let archived = log;
    println("{archived.len()}");
}
```
