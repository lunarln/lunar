# type::not_sync: a value that is not safe to share is shared between threads

A value whose type is not `Sync` would be read by several threads at once:
it is borrowed by the closure of `Scope::spawn` (or by the closure given to
`thread::scope`, which runs alongside its threads), put in a `Shared`, or
used as the type of a `static` (every thread sees a static), or used where a
bound says `T: Sync`.

`core::marker::Sync` is an auto trait: a type is Sync when all of its parts
are. Everything that is not `Send` is not Sync either, and neither is
`core::cell::UnsafeCell`, whose value may change behind a shared view. Types
built on a cell that guard their changes, such as `Mutex`, `RwLock` and the
atomics, promise `Sync` with an `unsafe impl`.

A `static mut` is exempt: every use of one is `unsafe` already, and keeping
threads from racing on it is up to the program. A `#[thread_local]` static
is exempt too: each thread has its own.

## Erroneous example

```lunar
use core::cell::UnsafeCell;

struct Counter {
    hits: UnsafeCell::<u64>,
}

static HITS: Counter = Counter { hits: UnsafeCell::new(0) };

fn main() {}
```

## Fixed

Use a type that is safe to change from several threads, such as an atomic:

```lunar
use std::sync::atomic::{AtomicU64, Ordering};

static HITS: AtomicU64 = AtomicU64::new(0);

fn main() {
    HITS.fetch_add(1, Ordering::Relaxed);
    println("{HITS.load(Ordering::Relaxed)}");
}
```

Or give every thread its own counter with `#[thread_local]`.
