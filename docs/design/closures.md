# Closures and `impl Trait`

Status: decided 2026-11-09, not built yet. Comes before the collections
and the parser ([trees-and-collections.md](trees-and-collections.md)).

## Syntax

As in Rust:

```lunar
m.update(word, |n| n + 1);
items.sort_by(|a, b| a.key.cmp(&b.key));
let add = |x: i32, y: i32|: i32 { return x + y; };
let hello = || println("hi");
```

Parameter types are optional when the expected `Fn` type gives them; a
return type comes after `:` and needs a block body. `||` is a closure with
no parameters where an expression starts, and logical or elsewhere.

## Captures

Inferred, as in Rust. A closure borrows what its body reads, borrows
mutably what it changes, and takes what it moves. `move |x| ...` takes
ownership of every capture instead (moving owners, copying Dull values).

```lunar
let mut total = 0;
items.for_each(|x| total += x);       // borrows `total` in-out
let c = move |x| x + offset;          // copies `offset` into the closure
```

A closure is a struct of its captures:

* a closure that borrows anything is a view: the borrow checker's view
  rules decide where it may go, so it cannot outlive what it borrows or
  escape the function;
* a `move` closure that captured only Dull values is Dull itself (it can be
  copied, stored in arrays);
* a `move` closure that captured an owner (`String`, an array) owns it: it
  moves, and dropping the closure drops the capture.

## `Fn`, `FnMut`, `FnOnce`

Three lang traits in core, by what a call may do to the captures: read
them, change them, or consume them (callable once). APIs ask for the
weakest they need:

```lunar
fn sort_by::<F: Fn(&T, &T): Ordering>(&mut self, f: F)
fn for_each::<F: FnMut(T)>(self, f: F)
fn update::<F: FnOnce(V): V>(&mut self, k: K, f: F)
```

The parameter types in `Fn(...)` carry Lunar's modes (`Fn(&String): usize`,
`FnMut(&mut i32)`), and calls through them lend arguments as calls to
functions do (`f(&a, &b)`). Calling a value of a closure type is written
like a function call: `f(x)`.

Closures are only passed through generic parameters (`F: Fn(...)`); there
is no `impl Trait` in parameter position.

## Escaping: `impl Trait` in return position

A function may return a value of a type it does not name:

```lunar
fn make_adder(n: i32): impl Fn(i32): i32 {
    return move |x| x + n;
}

fn evens(n: i32): impl Iterator::<i32> {
    return (0..n).filter(|x| x % 2 == 0);
}
```

The type is opaque, as in Rust: the body decides one concrete type (every
`return` must agree), callers see only the trait, and code is generated for
the concrete type (no indirection). It works for any trait, not only the
`Fn` traits. An escaping closure must be `move`, since a borrowing closure
cannot outlive the function.

## Not decided yet

* Whether named functions and `fn` pointers implement the `Fn` traits (as in
  Rust, so `items.sort_by(compare)` works). Expected: yes.
* Stored closures in struct fields go through generics
  (`struct Button::<F: Fn()> { on_click: F }`); trait objects (`dyn Fn`)
  would need `Box` and are out of scope.
