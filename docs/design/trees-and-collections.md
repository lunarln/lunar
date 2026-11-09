# Trees and collections

Status: built (2026-11-09): `core::arena`, `core::hash`,
`core::collections` and `core::intern`. Next: the parser in `craters/`.

## The problem

Array items must be Dull, and Lunar has no heap pointer, so a type cannot
contain itself and an array cannot hold `String`s or other arrays. Any
answer has to fit data stacks: every growable array or `String` owns a slot
of reserved address space (at least 64 KiB, 1 GiB by default), so "one
owner per node" does not scale to a parser's hundred thousand nodes.

## Decision: arenas now, `Box` later (maybe)

Trees and graphs are **arenas of Dull nodes linked by typed handles**. This
is Goose's "links within one array" idea as a library: nodes of a kind live
in one grow-only array, growth never moves them, and a link is a 4-byte
index. Children lists are packed runs in a shared list arena (Goose's
inline packing, applied only to lists). Nothing in the memory model
changes.

`Box::<T>` was considered and deferred. It would need a heap allocator in
the runtime, a new kind of owner, rules for reaching through it (field
access, lending, patterns; Lunar has no references and no `*` on
non-pointers), and arrays of owners (which reopen the Dull-items rule). It
also pulls toward heap strings and slices, since a boxed node cannot afford
a data-stack slot per name or child list. It will be revisited only if
arenas prove painful in practice.

## `core::arena`

```lunar
use core::arena::{Arena, Id, List};

pub struct Id::<T> { index: u32 }                 // a typed handle
pub struct List::<T> { start: u32, len: u32 }     // a packed run of items

enum Expr {
    Int(i64),
    Name(Ident),
    Binary(BinOp, Id::<Expr>, Id::<Expr>),
    Call(Id::<Expr>, List::<Id::<Expr>>),
}

let mut exprs: Arena::<Expr> = Arena::new();
let one = exprs.add(Expr::Int(1));
let sum = exprs.add(Expr::Binary(BinOp::Add, one, one));

let node = exprs.get(sum);                   // a copy
exprs.set(one, Expr::Int(2));                // replace
exprs.view_mut(sum)[0] = Expr::Int(3);       // in place
```

* `Id::<T>` only labels its index with the node type: an `Id::<Stmt>`
  where an `Id::<Expr>` is expected is a type error. (Generic parameters no
  field uses are allowed; a recursive type through a handle is not
  infinite, since `Id::<Expr>` stores only a `u32`.)
* Reads copy (`get`) or view (`view`, `view_mut`); writes replace (`set`)
  or update in place through a closure (`update(id, |n| ...)`).
* Handles are not tied to the arena that made them, and there are no
  generations: a parser's AST never frees nodes. Pools that do can add
  generations later.

## Names: symbols and spans

Tree nodes keep both an interned `Symbol` (for resolution: comparing names
is comparing integers) and a source span (for diagnostics):
`struct Ident { name: Symbol, span: Range }`. The interner is built on
`HashMap`.

## `core::hash` and `HashMap`

* `core::hash::Hash` (`fn hash(&self, h: &mut Hasher)`), implemented for
  the primitives and `str`, with `#[derive(Hash)]` as a macro in core like
  the other derives. One fixed fast hasher (not DoS-resistant).
* `HashMap::<K, V>`: unordered. Keys and values are Dull, like array items
  (so no `str` keys; an interner hashes the text and compares the bytes it
  stores itself). Resizing builds a new table and frees the old slot.
* `OrderedHashMap::<K, V>`: the same, iterating in insertion order
  (entries in a dense array plus an index table), for deterministic output.
* Operations: `insert` (returns the old value), `get` (a copy), `contains`,
  `remove`, `len`, iteration, and `update(k, |v| ...)` for changing a
  value in place. One-element views (`get_mut`) and `m[k]` place sugar
  were considered and rejected.
