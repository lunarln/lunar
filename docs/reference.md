# The Lunar Language Reference

## 1. Lexical structure

* Comments: `// line`, `/* block */` (block comments nest).
* Doc comments: `/// text` (exactly three slashes) documents the item,
  enum variant or struct field that follows; it is the attribute
  `#[doc = "text"]`, one per line. Elsewhere it is an ordinary comment.
* Identifiers: `[a-zA-Z_][a-zA-Z0-9_]*`. `_` alone is the placeholder.
* Keywords: `as break const continue crater dyn else enum extern false fn for
  if impl in let loop match mod mut pub return self Self static struct super
  trait true type unsafe use while`.
* Integers: `42`, `1_000`, `0xff`, `0o17`, `0b1010`, with optional suffix
  `i8 i16 i32 i64 isize u8 u16 u32 u64 usize`.
* Floats: `1.`, `1.5`, `2e10`, `1.5e-3`, optional suffix `f32 f64`.
  `0..10` is a range, not a float.
* Characters `'a'`, `'\n'`, `'\u{1F600}'`; strings `"..."` with escapes
  `\n \t \r \0 \\ \" \' \xHH \u{...}` and line continuations.
* Bytes `b'a'`, `b'\n'`, `b'\xff'`: a `u8` holding one ASCII character or
  one escape (`\xHH` up to `\xff`; no `\u`).

## 2. Items

```lunar
pub fn name::<T: Bound + Other, U>(x: &T, out: &mut U): ReturnType { ... }
unsafe fn raw(): i32 { ... }
#[test] fn it_works() { ... }

struct Point { pub x: i32, y: i32 }
struct Pair::<T>(T, T);
struct Unit;

enum Shape { Circle(f64), Rect { w: f64, h: f64 }, Empty }

trait Show: Display { fn show(&self): str; fn twice(&self) { ... } }
impl::<T: Dull> Show for Pair::<T> { ... }
impl Point { fn new(): Point { ... } }
trait Source { type Item: Light; const SIZE: usize; fn get(&mut self): Option::<Self::Item>; }
impl Source for Bytes { type Item = u8; const SIZE: usize = 1; ... }
fn sum::<I: Iterator::<Item = u64>>(it: I): u64 { ... }

use std::io::{print, println as say};
use crater::shapes::*;
mod shapes;            // loads shapes.ln or shapes/mod.ln
mod inline { ... }
const N: usize = 4;
static mut COUNT: u64 = 0;
static HITS: AtomicU64 = AtomicU64::new(0);   // `new` is a `const fn`
const fn square(x: u64): u64 { return x * x; }
unsafe impl Send for Handle {}
type Bytes = [u8; &];
extern "C" { fn puts(s: *u8): i32; #[link_name = "write"] fn c_write(fd: i32, b: *u8, n: usize): isize; }
```

Generic parameters are always introduced and applied with `::<...>`:
`Pair::<i32>`, `size_of::<T>()`, `impl::<T> ...`.

A **crater** is what Lunar compiles as one unit: a root file and every
module it loads with `mod`. Your program is a crater, and so are `core` and
`std`. In paths, `crater::` starts at the root module of the current
crater, `super::` at the parent module, and `self::` at the current one.

Items are private to their module (and its children) unless marked `pub`.
Struct fields are private unless `pub`.

### Constants, statics and `const fn`

`const` and `static` initializers are evaluated at compile time. They may
use literals, arithmetic, casts, `if`, arrays, tuples, other constants,
struct literals, enum variants, functions (as function pointers), field
reads, and calls of `const fn`s. A `const fn` is an ordinary function that
may also run at compile time; there its body may only bind `let name =
value;` and `return` a value (or end in one), using the same expressions.
That is how a type with private fields offers a constant:
`static LOCK: Mutex::<u64> = Mutex::new(0);`. Calling a function that is
not `const` in an initializer is `err(type::const_eval)`.

Statics cannot own data (no `String` or owning array). Every thread sees
the same statics, so a static that is not `mut` must be `Sync`
(`err(type::not_sync)`); its uses need no `unsafe`, and it may still change
through atomics or locks inside it. A `static mut` needs `unsafe` for every
use, and keeping threads from racing on it is up to the program.
`#[thread_local] static mut DEPTH: u32 = 0;` gives every thread its own
copy, starting from the initializer.

### Attributes

* `#[test]`: a test function, run by `lunar test`.
* `#[cfg(windows)]`, `#[cfg(unix)]`, `#[cfg(target_os = "linux")]`,
  `#[cfg(target_arch = "aarch64")]`, `#[cfg(test)]`, `not(...)`, `all(...)`,
  `any(...)`: conditional compilation. Any other name or `target_os` value
  is never set, so it is always false; the compiler warns about it
  (`warn(lint::unknown_cfg)`), e.g. `#[cfg(linux)]` for
  `#[cfg(target_os = "linux")]`.
* `#[link_name = "..."]` on extern functions.
* `#[thread_local]` on statics (above).
* `#[inline]`, `#[allow(...)]`, `#[doc = "..."]` are accepted.
* `#[lang = "..."]` and `#[intrinsic]` are used by the standard library.
* `#[derive(Name, ...)]` on a struct or enum runs those derive macros;
  `#[derive_macro]` marks a macro as a derive; `#[export]` makes an item a
  macro defines visible where it is invoked (see Macros).

### Macros

Macros are hygienic and declarative, and are items like functions
(`pub`, `use`, usable before their definition). The full design is in
[design/macros.md](design/macros.md).

```lunar
pub macro square($x:expr) {
    let t = $x;     // cannot clash with the caller's `t`
    t * t
}

macro max {
    ($x:expr) => { $x },
    ($x:expr, $($rest:expr),+) => {
        let a = $x;
        let b = max!($($rest),+);
        if a > b { a } else { b }
    },
}

let m = max!(3, square!(2), 1);
```

* Invoked as `name!(...)`, `name![...]` or `name!{...}` in expression,
  statement, item (also inside `impl` and `trait` bodies), pattern and type
  position. In expression position the expansion is a block, so it may
  contain statements.
* Matchers use `$name:fragment` with the fragments `ident`, `path`, `expr`,
  `ty`, `pat`, `block`, `stmt`, `item`, `lit`, `vis`, `meta`, `tt` and
  `generics`, and repetitions `$( ... ) sep op` with `*`, `+` or `?`.
* `$g:generics` matches `::<T: A, U>` (or nothing); in a body, `$g`,
  `$g.args` (`::<T, U>`) and `$g.bound(Trait)` (`::<T: A + Trait, U: Trait>`).
* Hygiene: names in a macro's body resolve where the macro is defined, its
  `let`s are invisible to the caller and vice versa, and items it defines
  are hidden from the caller unless marked `#[export]`.
* A derive is a macro marked `#[derive_macro]`; `#[derive(Name)]` passes it
  the struct or enum's tokens, and it may only add items (an `impl`).
* Macro errors are `macro::...`; an expansion's code is shown as a file
  named after the macro (`<square!>`).

## 3. Types

| Type | Meaning |
|---|---|
| `i8`…`i64`, `isize`, `u8`…`u64`, `usize` | integers (`usize` is 64-bit) |
| `f32`, `f64`, `bool`, `char` | primitives |
| `()`, `(A, B)` | tuples |
| `[T; 4]` | fixed array, stored inline |
| `[T; _]` | fixed array whose length is inferred |
| `[T]` | owning array, sized at construction then frozen |
| `[T; ..+]` | owning array that can grow but never shrinks |
| `[T; -..+]` | owning array that can grow and shrink |
| `[T; &]`, `[T; &mut]` | read-only / mutable slices (views) |
| `+T` | `&mut` parameters and receivers only, where `T` owns an array or string (or contains one): the argument only grows, so the caller's views into it stay valid (see §4) |
| `str` | UTF-8 view (like Rust's `&str`) |
| `dyn Trait`, `dyn mut Trait` | trait objects: a shared / changing view of a value of any type implementing `Trait` (see §7) |
| `String` | owned UTF-8 text (a `[u8; -..+]` with a UTF-8 invariant) |
| `FmtArgs` | parameter only: an interpolated string literal |
| `*T`, `*mut T` | raw pointers (dereferencing is unsafe) |
| `fn(A, &B, &mut C): R` | function pointers; parameter modes are part of the type |
| `!` | the never type (return type of functions that never return) |

### Dull, Light and pointy types

Two traits are implemented **automatically by the compiler** and cannot be
implemented or `!`implemented manually (`err(trait::dull_impl)`):

* `core::ptr::Dull`: the type is not a pointer, slice, string or array and
  contains none. All other types are *pointy*.
* `core::ptr::Light`: the type owns no array or string. Dull types are
  Light (`Dull: Light`), and so are views (`str`, `[T; &]`, `dyn Trait`,
  raw pointers) and anything built only from Light types. `String`, owning arrays and
  types containing them are not.

**Array and slice items must be Light:**

```text
err(type::array_owner_items): Arrays and slices cannot hold items that own arrays or strings.
help: add Light to T's bounds: `T: Light`
```

So `[str; ..+]`, `[[u8; &]; 4]` and an `Arena` of nodes holding `str` names
are fine, and the borrow checker tracks what the views among the items
point into: an array of views borrows from everything its items point
into, `push`ing a view adds to that, and an item read out of it points
where the item points.

Light values are copied implicitly. Owning values (owning arrays, `String`,
and types containing them) are **moved**.

`core::cell::UnsafeCell::<T>`, a value that may change behind a shared view
(through the raw pointer `get` gives), is neither Dull nor Light: it and the
types built on it (`Mutex`, the atomics) are moved, never copied, so no copy
is ever read while another thread changes the original.

### Send, Sync and Detached

Three more traits are implemented by the compiler from a type's parts. They
decide what may cross between threads (§6a):

* `core::marker::Send`: a value may move to another thread. Everything is
  Send except raw pointers, mutable slices (`[T; &mut]`: they are copied
  freely, so two threads could write through the same one), `FmtArgs`,
  shared slices `[T; &]` whose items are not Sync, closures that change a
  variable they borrow, `dyn` views (unless `dyn Trait + Sync`, below), and
  types holding one of those.
* `core::marker::Sync`: threads may share views of a value (read it at the
  same time). Everything that is not Send is not Sync, and neither is
  `UnsafeCell`.
* `core::marker::Detached`: the value holds no views: no `str`, slice or
  closure that borrows, at any depth. Owners, Dull values and raw pointers
  are Detached. A Detached value points into nothing it does not own, so it
  may outlive every local variable. A generic `T: Detached` holds no views
  for the borrow checker either.

A closure borrowing a variable is Send and Sync if the variable's type is
Sync; one owning a capture is Send (Sync) if the capture is.

A `dyn Trait` is a shared view, like `[T; &]`: it is Send and Sync when the
value it views is Sync. Its type does not say which type that is, so only
`dyn Trait + Sync` is (the value is checked to be Sync where it is first
viewed as `dyn`). `+ Send` records that the value is Send; it makes no view
Send by itself. A `dyn mut Trait` is like `[T; &mut]`: neither Send nor
Sync. No `dyn` is Detached.

A struct or enum may promise `Send` or `Sync` with `unsafe impl` (for raw
pointers or cells it guards itself), or give them up with a negative impl
(`impl !Send for Handle {}`). Its impls then decide alone, and its fields no
longer count: `unsafe impl::<T: Send> Sync for Mutex::<T> {}`. Implementing
either without `unsafe` is `err(trait::unsafe_impl)`, and only for a struct
or enum of the same crater (`err(trait::auto_impl)`). `Detached`, like
`Dull` and `Light`, cannot be implemented by hand. An unsatisfied bound on
one of them is `err(type::not_send)`, `err(type::not_sync)` or
`err(type::not_detached)`, with notes down to the part that is the reason.

## 4. Memory model: data stacks

Every owning array (`[T]`, `[T; ..+]`, `[T; -..+]`, `String`) is a small
handle `{ ptr, len, cap }` pointing at its own *data stack*: a slot of
address space whose pages are committed as the array grows. By default a
slot is 1 GiB, so an array can hold at most 1 GiB; growing past its slot is
a fatal `data stack overflow`.

`capped(n)` limits an empty array (or `String`) to exactly `n` elements
(bytes for a `String`) and gives it the smallest slot that holds them, from
64 KiB up to 1 TiB, so more arrays fit and large ones can exceed 1 GiB:

```lunar
let mut log: [u8; ..+] = [].capped(1 << 20);       // at most 1 MiB
let mut big: [f64; ..+] = [].capped(1 << 32);      // 32 GiB of address space
let mut name = String::new().capped(64);
```

The cap is part of the value, not the type: a capped `[u8; ..+]` is still a
`[u8; ..+]`. Pushing past it is a fatal `data stack overflow`. Arrays that
never grow (`[T]`) get a slot sized to fit automatically.

All slots come from one region reserved at startup (up to 64 TiB, so up to
65,536 arrays can be alive at once). Taking and returning a slot never calls
the operating system. On Linux and macOS the region is usable up front and
the kernel supplies pages on first touch, so allocation makes no system
calls after startup; on Windows, memory is committed only when a slot grows
past the most it has ever held. Freed slots keep their memory for reuse, up
to 64 MiB in total.

* **Growing never moves elements.** Slices taken from a `[T; ..+]` stay
  valid after any number of `push`es.
* **Shrinking and replacing invalidate slices.** A `&mut` parameter (or
  `&mut self`) may do anything to its argument: grow it, shrink it
  (`pop`, `truncate`, `clear`), or replace an owner inside it
  (`self.items = [];` frees the old array). So lending a value in-out ends
  every view into it: using one after the call is
  `err(borrow::shrink_while_borrowed)`. The same holds for calling a
  closure that changes what it borrowed.
* **`&mut +T` promises to only grow.** A parameter declared `&mut +T` (or a
  receiver `&mut +self`) is never shrunk or replaced, so the caller's views
  survive the call. `push`, `extend`, `reserve`, `String::push_str`,
  `String::write`, `Arena::add` and `Interner::intern` are `+`, and so is
  the `out` of `Display::fmt` and `Debug::debug`, which `String::write`
  calls for each hole. Methods
  that move what views would see are not, even when nothing is freed:
  array `insert` shifts elements, and `HashMap::insert` may rebuild its
  table, so a loop over a map cannot insert into it:

  ```lunar
  let s = a[0..2];
  a.push(9);          // push is `&mut +self`: `s` stays valid
  a.pop();            // pop is `&mut self`: `s` ends here
  use(s);             // err(borrow::shrink_while_borrowed)

  impl Stack {
      fn push_twice(&mut +self, v: i32) { self.items.push(v); self.items.push(v); }
  }
  ```

  The compiler holds the function to it: inside, a `+` parameter may only
  be read, changed in its Dull parts, and passed on to other `+`
  parameters. Shrinking it, replacing an owner in it, or lending it to a
  plain `&mut` parameter is `err(borrow::grow_only)`.
* `+` always comes with `&mut` (`err(type::grow_without_mut)`), appears
  only on parameters (`err(type::grow_outside_param)`), and needs a type
  that owns an array or string (`err(type::grow_no_owner)`). A `+T`
  parameter accepts any `T` argument.
* Views are tracked per variable, not per field: a view into any part of a
  value is ended by lending the whole value in-out, even if the callee only
  touches a disjoint field. This is conservative but sound; tracking
  disjoint fields separately is planned. A view taken from a `str` or slice
  *field* points where that field points, not into the struct, so it
  survives changes to the struct.
* Owners are freed when they go out of scope (also on `return`, `break`,
  `continue`). Moving out of a variable makes it unusable
  (`err(borrow::use_after_move)`).
* Parameters take ownership unless marked (see *Parameters* below). A
  borrowed parameter cannot be moved out of
  (`err(borrow::move_out_of_borrow)`).
* Views cannot outlive what they point into (`err(borrow::outlives_root)`)
  and cannot be returned when they point into the function's own locals
  (`err(borrow::escapes_function)`).

### Parameters

How a function receives each argument is part of its signature:

| Parameter | Meaning |
|---|---|
| `x: T`, `self` | **takes ownership**: the caller's value is moved into the call (or copied, if it is Dull), and the function frees it |
| `mut x: T`, `mut self` | the same, and the function may change its own value |
| `x: &T`, `&self` | **borrows**: the caller keeps ownership; the function can read it but not keep or move it |
| `x: &mut T`, `&mut self` | **in-out**: the caller passes a mutable place and sees the changes |
| `x: &mut +T`, `&mut +self` | in-out, and only grows it (see above) |
| `kept x: T` (any mode) | the function may store views from this argument into its in-out arguments |

A function that stores a view it was given (a `str`, a slice, or a view
into a borrowed owner) into one of its in-out arguments declares that
parameter `kept`: `fn set(&mut self, kept s: str) { self.rest = s; }`.
Callers then treat the in-out arguments of the call as pointing into the
kept arguments, so `h.set(s.as_str())` makes `h` borrow from `s`. Storing a
view from a parameter that is not `kept` is
`err(borrow::escapes_via_param)`; arguments that are not kept may be
short-lived. An implementation of a trait method may only keep what the
trait's declaration keeps. Closures are treated as keeping every argument
and capture. `kept` is a word only in this position.

Call sites say what happens to each argument. A plain argument is moved
(or copied), `&x` lends `x` to a borrowing parameter, and `&mut x` lends it
to an in-out parameter:

```lunar
fn new(name: String, age: u32): Person {   // keeps `name` without copying it
    return Person { name, age };
}
fn len(s: &String): usize { return s.len(); }
fn bump(x: &mut i32) { x += 1; }

let p = new(name, 30);   // `name` is moved
let n = len(&p.name);    // borrowed
let mut k = 1;
bump(&mut k);            // k == 2
```

The marker must match the parameter: a missing `&` or `&mut`, or one where
the parameter takes ownership, is an error. Using a moved variable
afterwards is `err(borrow::use_after_move)`. Three cases need no marker:

* a method's receiver: `v.push(1)` lends `v` to `&mut self` (the method's
  signature says so);
* a parameter that already is a borrow, passed on: inside
  `fn f(s: &String, out: &mut String)`, `len(s)` and `bump_all(out)`;
* an in-out parameter passed on where only a borrow is needed: `len(out)`.

`&T` and `&mut T` exist only on parameters; they are modes, not types you
can store. `str` and `[T; &]` are the first-class views. Borrowing a Dull
type is pointless (`warn(lint::needless_borrow)`), and a `mut` parameter
that is changed but never read again probably meant `&mut`
(`warn(lint::mut_param_not_read)`).

Borrowed parameters are passed by address, so a view into a borrowed value
points into the caller's data and may be returned:
`fn first_two(a: &[i32; 4]): [i32; &] { return a[0..2]; }`.

#### What a returned view points into

Which arguments a call's result may point into is part of the called
function's signature, like `kept`: callers are checked against the
signature, never the body, so changing a body cannot break them. A `from`
clause after the return type names the parameters (or `self`). Here the
result points into `x` only, so `y` may change or move while it is in use:

```lunar
fn first(a: &String, b: &String): str from a {
    if b.len() > 3 { return a.as_str()[1..]; }
    return a.as_str();
}

let v = first(&x, &y);
y.clear();               // fine: `v` does not point into `y`
x.clear();               // err(borrow::shrink_while_borrowed)
println("{v}");
```

* A parameter can be named if a view could come from it: a borrowed one
  (`&T`, `&mut T`, `&self`, ...), or one that holds views itself (a `str`,
  a slice, a struct holding one, a generic `T` not bounded by `Dull`).
  Naming another, or giving a `from` clause to a function whose return
  type holds no views, is `err(type::from_without_views)`.
* `from static` says the result points into no parameter, only into
  static data such as string literals:
  `fn kind(a: &String, b: &String): str from static`.
* Without a clause, a method that borrows its receiver (`&self`, `&mut
  self`, `&mut +self`) returns views into `self` only, whatever its other
  parameters. Any other function returns views into every parameter a view
  could come from; with just one, that is the one. A return type that
  holds no views needs nothing.
* The body is checked against the signature: every value it returns (its
  tail, its `return`s, and through them the `break`s of loops and the arms
  of `if` and `match`) may point only into the parameters declared, or
  `err(borrow::escapes_via_return)`.
* At a call, a result may point into the declared arguments' own data
  (and wherever their views point). A function whose declared return type
  has no view of its own (`pop(&mut self): Option::<T>`) never points into
  its arguments' own data, only where their items point.
* A trait method's clause is part of the trait: a call on a generic
  receiver (`t.name()` with `T: Named`) uses the trait's, and a call on a
  concrete type the implementation's (or the default body's). An
  implementation may point into no more than the trait declares
  (`err(trait::signature)`).
* Calls through function pointers may point into every argument; their
  types have no `from` clause.
* Closures are not part of any signature, so a closure's result is
  inferred from its body. Calling one gives a view into a variable it
  borrows only if its body returns one, so a closure may capture a
  borrowed parameter and return a view of it
  (`let tail = |i: usize| s.as_str()[i..];`), and a result that does not
  come from an argument does not borrow it. A closure whose type is not
  known (a parameter `f: F` with `F: Fn(): str`) may return a view into
  everything it borrows.

When a borrow error comes from a call's result, it points at the argument:
"the value returned by `first` points into `x`". `from` is a word only
after a return type: functions, methods and parameters may still be named
`from`.

Function pointer types record each parameter's mode, written as in
signatures: `fn(&String): usize`, `fn(&mut i32)`, `fn(String): usize`. A
function converts to a pointer type with the same modes, and a call through
the pointer passes, borrows and moves arguments exactly as a direct call
would.

### Mutation

`let` bindings are immutable, `let mut` bindings mutable.

## 5. Expressions and statements

Lunar is expression based. Statements end with `;`. A block's final
expression without `;` is its value (a *tail expression*). Block-like
expressions (`if`, `match`, loops, blocks) need no `;` as statements.

```lunar
let x = if a > b { a } else { b };
let kind = match shape { Shape::Circle(r) => r, _ => 0.0 };
let v = loop { if done() { break 42; } };
for i in 0..10 { }       // integer ranges, also `a..=b`
for x in array { }       // arrays and slices (by value, Dull items)
for c in s.chars() { }   // anything implementing Iterator (its Items)
for (i, w) in s.split(" ").filter(|w| w.len() > 1).enumerate() { }
while let Some(x) = it.next() { }
if let Some(v) = maybe { }
let Some(v) = maybe else { return; };  // `v` is bound for the rest of the block
let n = text.find("x")?; // `?` on Option and Result
let size = match shape { Shape::Circle(r) | Shape::Square(r) => r, _ => 0.0 };
```

* Patterns go in `match` arms, `if let`, `while let`, `let`, `let .. else`
  and `for`. A `|` pattern matches if any of its alternatives does, at the
  top or nested (`Some(A(x) | B(x))`). Its alternatives may bind variables:
  every alternative binds the same names (`err(name::unbound_in_alternative)`),
  each with the same type (`err(type::mismatch)`) and the same `mut`
  (`err(name::inconsistent_binding)`), and the code after the pattern sees
  one variable, whichever alternative matched. A guard (`if`) sees it too.
* A `let` takes a pattern that always matches; that includes `|` patterns
  whose alternatives cover the type together (`let (Ok(v) | Err(v)) = r;`).
* `let pattern = value else { ... };` takes a pattern that may fail to
  match. When it does not match, the `else` block runs and must leave
  (`return`, `break`, `continue`, or a call returning `!`):
  `err(type::let_else_falls_through)`. It cannot see the pattern's bindings.
  A pattern that always matches gets `warn(lint::irrefutable_let_else)`.
* Indexing `a[i]` is bounds checked; `a[i..j]`, `a[..]`, `a[i..=j]` slice.
* Struct literals `Point { x: 1, y }`; tuples `(1, 2)`, `t.0`.
* Array literals `[1, 2, 3]`, repeats `[0.; 8]` (the count may be a runtime
  value for owning arrays).
* Casts with `as` between numeric types, `bool`/`char` to integers, `u8` to
  `char`, and between pointers and `usize`. `x as dyn Trait` views `x` as a
  trait object (§7).
* Operators on user types go through the traits in `core::ops` and
  `core::cmp` (`Add`, `Sub`, ..., `Eq` for `==`, `Ord` for `<`); the
  arithmetic traits' `Output` is the result's type (§7).
* Integer overflow panics in debug builds and wraps in `--release`.
  Division by zero always panics.

### Closures

`|params| body` is a closure; `|x: i32|: i32 { ... }` gives a return type
(with a block body), `||` takes no parameters. Parameter types may be left
out where the expected `Fn` type supplies them.

```lunar
let k = 10;
let plus_k = |x: i32| x + k;          // borrows `k`
let mut count = 0;
let mut bump = || count += 1;         // borrows `count` in-out
let name = String::from("x");
let give = move || name;              // takes `name`
```

A closure captures the variables its body uses: it borrows what it reads,
borrows in-out what it changes, and takes what it moves. `move` takes every
capture (moving owners, copying Dull values). A closure that borrows is a
view: it cannot outlive what it borrows, and what it borrows cannot be
moved while it is used.

What a call may do to the captures decides which of the core traits a
closure implements: `Fn` (reads them), `FnMut` (changes them; the closure
must be `let mut`) or `FnOnce` (gives one away; calling consumes the
closure). Functions are generic over them: `fn apply::<F: Fn(i32): i32>(f:
F, x: i32)`; the parameter types in `Fn(...)` carry modes (`Fn(&String)`,
`FnMut(&mut i32)`). Named functions and function pointers implement all
three. A closure given where `Fn(&mut +String)` is expected takes that
parameter as `&mut +`, so it may only grow it, and calling `f(s)` through
such a bound keeps the views into `s`, as for a `&mut +` parameter of a
function.

A function can return a closure (or any value) without naming its type:
`fn make_adder(n: i32): impl Fn(i32): i32 { return move |x| x + n; }`.
Callers see only the bounds after `impl`; the body returns one concrete
type. A returned closure must be `move`.

### Format strings

A string literal is interpolated only when it is passed to a `FmtArgs`
parameter, such as `println`, `print`, `panic`, `String::write` or
`core::fmt::format`. `{expr}` formats `expr` with `Display`; `{{` and `}}`
are literal braces. Everywhere else a string literal is a plain `str`.

A type is made printable by implementing `Display` (or `Debug`, for
`{expr:?}`), which appends to a grow-only `String`, so that `s.write(..)`
leaves views into `s` valid:

```lunar
impl Display for Point {
    fn fmt(&self, out: &mut +String) {
        out.write("({self.x}, {self.y})");
    }
}
```

A hole may contain string literals: `println("{m.get("k")} {greet("moon")}")`.
Because any string literal may turn out to be a format string, the lexer
reads a `{` as opening a hole only when what follows looks like an
expression: brackets balance up to a matching `}`, and each `"` inside
comes where an operand can begin (after an operator, `(`, `[`, `{` or `,`,
not first in the hole) and closes on the same line. Otherwise the `{` is
text, so plain strings such as `"{"`, `"} else {"` and `"{\"k\": 1}"` mean
what they always did. A `:` outside brackets (not part of `::`) starts the
hole's spec, which runs to the next `}` on the line.

#### Format specs

After the value, a hole may have a format spec in Rust's syntax:

```text
{value:[[fill]align][sign]['#']['0'][width]['.' precision][type]}
```

| Part | Meaning |
|---|---|
| `fill` | any character but `{`, `}` and `:` (which would read as `::`), before an `align` (default a space); `"` and `\` are written `\"` and `\\` |
| `align` | `<` left, `^` centered (an odd pad puts the extra one on the right), `>` right. The default is right for numbers and left for anything else |
| `+` | a sign on non-negative numbers too: `+5`, `+0`, `+inf` (never `+NaN`). `-` is accepted and means nothing, as in Rust |
| `#` | with `x`, `X`, `o` and `b`: the prefix `0x`, `0o` or `0b` (`0x` with `X` too) |
| `0` | pad numbers with zeros after their sign and prefix: `{n:+08}` is `+0000042`, `{n:#010x}` is `0x0000002a`; overrides `fill` and `align` |
| `width` | the least number of characters (Unicode scalar values, not bytes) to show; a number, or `name$` for a `usize` variable `name` |
| `.precision` | for floats, the digits after the point, rounded correctly (ties to even on the exact value, as in Rust: `{0.125:.2}` is `0.12`); for a `str`, `String`, `char` or `bool` shown with `Display`, how many characters to keep; ignored for integers. A number, or `.name$` |
| `type` | none: `Display`; `?`: `Debug`; `x` and `X`: hexadecimal; `o`: octal; `b`: binary; `e` and `E`: scientific (`1.2345e3`) |

```lunar
let n = -42;
let pi = 3.14159265;
let w: usize = 10;
println("[{n:>6}] [{n:*^7}] [{n:+}] [{n:#010x}] [{pi:.2}] [{pi:>w$.3}] [{pi:e}]");
// [   -42] [**-42**] [-42] [0xffffffd6] [3.14] [     3.142] [3.14159265e0]
```

`x`, `X`, `o` and `b` take integers, shown as the bits of their own
width (`{-1i8:x}` is `ff`), and `e` and `E` take floats; `+` and `0` are
for numbers, and a precision for floats and (with `Display`) text. Each
other use is `err(type::bad_format_spec)`, at the part of the spec that
does not fit; a malformed spec (an unknown type, a positional `1$` or `.*`,
`{x:#?}`, which is not supported) is `err(syntax::bad_format)`. A spec with
a fill and alignment but no width is a plain hole, and so is `{x:}`.

The value is written first, with `Display` or `Debug` as usual (or by the
compiler, for the radix and exponent types and float precision), and then
padded in place: a width pads the whole text a value writes, whatever its
type, so `{p:>10}` pads a `Point`'s `(1, 2)` and `{v:>12?}` pads the whole
`[1, 2, 3]`. (Rust instead leaves width to each implementation, which may
ignore it or, for `Debug` of collections, apply it to each element.) The
`Display` and `Debug` traits do not see the spec, so a precision cannot
reach the fields of a struct or the elements of an array. A hole without
a spec compiles to exactly the call it always did.

## 6. Unsafe code

Unsafe operations: calling an `unsafe fn` or an `extern` function,
dereferencing a raw pointer, and accessing a `static mut`. They must appear
inside `unsafe { ... }`:

* `err(unsafe::outside_unsafe)`: the operation is outside any unsafe block.
* `err(unsafe::in_unsafe_fn)`: the operation is inside an `unsafe fn` but
  not in an `unsafe` block. Marking a function `unsafe` only makes it unsafe
  to call; its body gets no implicit unsafe context.

Inside `unsafe`, the program is responsible for memory safety. Reading or
writing outside a live value (for example, past the length of an array
through a raw pointer) is **undefined behaviour**: it may crash, corrupt
other data, or appear to work. In particular, arrays sit next to each other
in memory and the space past an array's length is not guaranteed to fault,
so an overrun is not reliably caught at run time.

`unsafe impl Send for T {}` and `unsafe impl Sync for T {}` are promises the
compiler cannot check (§3); no other impl may be `unsafe`.

## 6a. Threads

`std::thread` starts threads of the operating system. Lunar's rules keep
them from racing on data and from viewing memory that is gone:

* `thread::spawn(f)` runs `f` on a new thread that may outlive its caller,
  so `f` and its result must be `Send` and `Detached`: a `move` closure of
  owned values. Borrowing closures and views are `err(type::not_detached)`.
  `h.join()` waits for it: `Result::<T, Panic>`.
* `thread::scope(|s| { s.spawn(|| ...); })` starts threads that may borrow
  from the caller: all of them finish before `scope` returns. The closure
  given to `scope` runs alongside its threads, so it must be `Fn` (it may
  not change what it captures) and `Sync`; each thread's closure must be
  `Send`, so what it borrows must be `Sync`. A thread may borrow what the
  scope's closure borrows, not that closure's own variables, which are
  freed before the threads are waited for
  (`err(borrow::scoped_thread_borrow)`). `s.spawn_with(arg, |a| ..)` moves
  `arg` (a slice of the work, say) into the thread.
* Data threads share is changed only through locks and atomics, which are
  used through closures, since Lunar has no references to keep a lock held:
  `m.lock(|v| v += 1)`. A lock's closure gets the value in-out (`&mut T`) and
  its result, which must be `Detached`, is what `lock` returns, so no view of
  locked data outlives the lock. The values in locks and cells are
  `Detached` too.
* A panic on a thread other than the main thread ends only that thread, on
  the spot (no unwinding: what it owned stays allocated). It prints its
  message like the main thread does, `join` returns it as an `Err`, and the
  locks it held are poisoned: `lock` then panics, `lock_checked` gives the
  value with a `PoisonError`. A panic on the main thread ends the program.
* The runtime is thread-safe: arrays may be made on one thread and freed on
  another.

## 7. Traits

Traits work like Rust's: `trait Name::<Params>: Supertraits { fn m(self); }`
with default methods, static functions (`T::default()`), generic and blanket
impls (`impl::<T: Dull> Clone for [T; ..+]`), and negative impls only for
the auto traits `Send` and `Sync` (`Dull`, `Light` and `Detached` cannot be
negated). An implementation's methods
take their parameters exactly as the trait declares, `+` included: an
`out: &mut String` where the trait says `out: &mut +String` is
`err(trait::signature)`. Trait method calls are resolved
statically and monomorphized, except through trait objects.

### Associated types and constants

A trait may declare types and constants that each impl gives:

```lunar
trait Source {
    type Item;                       // each impl says what it is
    type Raw: Light + Debug;         // with bounds every impl's type meets
    type Label = str;                // with a default an impl may keep
    const SIZE: usize;               // each impl gives a value
    const NAME: str = "source";      // or keeps the default
    fn get(&mut self): Option::<Self::Item>;
}

impl Source for Counter {
    type Item = u32;
    type Raw = u64;
    const SIZE: usize = 4;
    fn get(&mut self): Option::<u32> { ... }
}
```

* An impl gives every associated type and constant the trait declares
  without a default (`err(trait::missing_item)`), and no other
  (`err(trait::extra_item)`). Its types meet the trait's bounds
  (`err(type::unsatisfied_bound)`); its constants have the declared type
  (`err(trait::signature)`). Associated types take no generic parameters.
  Outside a trait, a constant needs its value and a type alias its type
  (`err(syntax::missing_value)`). An inherent impl may have constants
  (`impl Limits { const MAX: usize = 16; }`), not types.
* `T::Item` and `Self::Item` name the associated type of a generic
  parameter or `Self` through its bounds (and their supertraits): a
  *projection*. If no bound has one, it is `err(name::no_assoc)`; if two do,
  `err(trait::ambiguous_assoc)`. `<T as Trait>::Item` names the trait, and
  works for any type (`<Counter as Source>::Item` is `u32`); `Counter::Item`
  is not allowed, as several traits could give it one. Inside an impl of a
  trait, `Self::Item` is the impl's own type.
* Constraints say what associated types are, after a trait's arguments:
  `I: Iterator::<Item = u64>`, `C: Convert::<u8, Out = u16>`, `impl
  Iterator::<Item = str>` (a result), `dyn Iterator::<Item = u8>` (§ Trait
  objects) and supertraits (`trait Int: Add::<Output = Self>`, which every
  impl of `Int` must meet). A constraint is a parameter of the caller's
  choosing: `fn f::<K, V, I: Iterator::<Item = (K, V)>>(it: I)` learns `K`
  and `V` from the iterator given. A type that does not meet one is
  `err(type::assoc_mismatch)`.
* A projection is the type it stands for as soon as that is known: from a
  constraint in scope, an `impl Trait`'s or `dyn`'s constraints, or the
  impl for a known type. Otherwise (`I::Item` for a generic `I` without a
  constraint) it is a type of its own, like a generic parameter, which the
  trait's bounds on it describe: `type Item: Light;` makes `I::Item` Light.
* `T::N`, `Self::N`, `Type::N` and `<T as Trait>::N` read an associated
  constant: in generic code, the value for each instance. They can be used
  in the initializers of constants and statics and in array lengths when
  the type is known there (`const BOTH: u32 = Square::SIDES + <Tri as
  Shape>::SIDES;`, `[0u8; Limits::MAX]`). A trait's default value cannot
  depend on `Self`.
* The operator traits (`core::ops::Add`, ...) have `type Output`, the type
  of `a + b`; `a += b` needs `Output` to be `a`'s type. `core::str::FromStr`
  has `type Err`, what `parse` gives on failure.

### Trait objects

`dyn Trait` is a view of a value of any type implementing `Trait`: a
pointer to the value and a pointer to its type's *vtable* for the trait, a
table of the type's methods. Calls through it go through the table, so one
array can hold views of different types:

```lunar
trait Shape { fn area(&self): f64; }

let c = Circle { r: 1.0 };
let r = Rect { w: 2.0, h: 3.0 };
let shapes = [c as dyn Shape, r as dyn Shape];
for s in shapes { total += s.area(); }
```

* A `dyn` is made only with an explicit `x as dyn Trait`, never implicitly
  (at a call, a `return` or in an array literal it is `err(type::mismatch)`).
  `x`'s type must implement the trait (`err(type::unsatisfied_bound)`).
* It is a view, like a slice, and is Light: it is copied, stored in arrays
  and fields, passed and returned like a `str`. It never owns its value and
  is never Detached. It points at the place `x`: a variable, a field, an
  item of an array, or the caller's value behind a borrowed parameter. The
  borrow checker treats it like a closure borrowing that place: it cannot
  outlive it (`err(borrow::outlives_root)`; a view of a temporary dies with
  its statement), the place cannot be moved while it is used
  (`err(borrow::move_while_borrowed)`), and it is returned only from a
  parameter the caller lent, as its signature says (`from`). The variable
  may still be assigned and changed: the `dyn` sees the new value.
* `dyn Trait` gives shared access: only `&self` methods can be called
  through it. `dyn mut Trait`, made from a mutable place, may also call
  `&mut self` and `&mut +self` methods (else `err(type::immutable)`). Like
  lending the value in-out, a `&mut self` call through a `dyn mut` may
  shrink or replace the value, so it ends views into it
  (`err(borrow::shrink_while_borrowed)`), and so does passing a `dyn mut`
  to a call, which may make such calls. A `&mut +self` call only grows the
  value and keeps them. A `dyn mut` cannot be stored inside the value it
  views (`err(borrow::views_itself)`).
* A `dyn mut Trait` converts implicitly to a `dyn Trait`, and `+ Send` and
  `+ Sync` may be dropped implicitly. `d as dyn Super` views a `dyn Sub` as
  one of its supertraits (upcasting).
* The supertraits' methods can be called through a `dyn`, and generic
  traits take their arguments: `dyn Convert::<u8>`. Format holes call
  `Display` and `Debug`, so `{x}` works for `x: dyn Display`.
* A `dyn` names every associated type of its trait and its supertraits:
  `dyn Iterator::<Item = u8>` (`dyn Iterator` alone is `err(type::bad_dyn)`).
  The value viewed must have those (`err(type::assoc_mismatch)`).
* Associated constants are read from the value a `dyn` views, through its
  vtable: `d.SIDES`, like a field (a `dyn` has no fields), for the constants
  of its trait and supertraits. `<dyn Shape>::SIDES` cannot work, since each
  value viewed may have its own.
* `dyn Trait` implements `Trait` and its supertraits, so it can be passed
  to generic code; a shared `dyn Trait` implements only traits whose
  vtable methods (with their supertraits') all take `&self`, so generic
  code cannot change a value through it.
* The crater that defines a trait may give `dyn Trait` methods of its own
  (`impl dyn Shape { fn double_area(&self): f64 { ... } }`); they are found
  on `dyn mut Shape` and `dyn Shape + Sync` too.
* Auto traits follow the trait: `dyn Trait + Send + Sync` (§3); `Dull`,
  `Light` and `Detached` cannot follow it, and a `dyn` names exactly one
  trait besides them (`err(type::bad_dyn)`).

A trait can be used as `dyn` (is *dyn compatible*) when every method of it
and of its supertraits without a default body takes `&self`, `&mut self` or
`&mut +self`, has no generic parameters, and mentions `Self` nowhere else in
its signature (other than as `Self::Item`, which the `dyn` type names);
its supertraits do not take `Self` as an argument; and its associated
constants' types do not mention `Self`
(`err(trait::not_dyn_compatible)`). Methods with a default body that break
these rules are not in the vtable: calling one on a `dyn` runs the default
body with `Self` being the `dyn` type itself (which needs `dyn Trait` to
implement `Trait`). Closure traits (`Fn`, ...) cannot be used as `dyn` yet.

The vtable of a type for a trait is one constant per program (shared by all
craters): a null slot reserved for drop glue (for future owned trait
objects), the type's size and alignment, the vtables of the supertraits
(for upcasting), then the methods, trait by trait (the trait itself, then
its supertraits depth first), each in declaration order, then the
associated constants in the same order, each word a pointer to the
constant's value for the type.

Method calls *autoslice*: a method called on `[T; -..+]` also finds the
methods of `[T; ..+]`, `[T]`, `[T; &mut]` and `[T; &]`, in that order, and a
`String` receiver finds `str` methods.

## 8. Diagnostics

Every diagnostic has a scoped code: `err(scope::name)`. Scopes:
`syntax`, `name`, `type`, `trait`, `borrow`, `unsafe`, `lint`, `build`.

* `lint` codes are warnings, printed as `warn(lint::name)`:
  `unused_variable` (assigning to a variable does not count as using it),
  `unused_mut`, `mut_param_not_read`, `unreachable` (code after `return`,
  `break`, `continue`, a call returning `!` or a `loop` without `break`),
  `needless_borrow`, `unknown_cfg` and `irrefutable_let_else`. The first four are reported only
  for functions without errors, and never for the standard library.
* `build` codes are reported after the program has been checked, and are
  not about a place in the source: `no_main` (by `build`, `run` and
  `emit-llvm`; `lunar check` does not require `main`), `llvm` (code
  generation), `link` (with the linker's output as a note), `io` and
  `missing_lang_item` (only with a modified standard library).

`lunar explain scope::name` (or the pasted `err(scope::name)`) explains a
code in detail, with an erroneous and a fixed example; `lunar explain`
alone lists every code. The explanations are the files in
[docs/errors](errors), one per code.

## 9. Standard library overview

* `core::ptr::Dull`, `core::ops`, `core::cmp::{Eq, Ord, Ordering, min, max,
  Reverse}`, `core::clone::Clone`, `core::default::Default`,
  `core::fmt::{Display, Debug, write, format}`, `core::option::Option`,
  `core::result::Result`, `core::num::{Int, Float}`,
  `core::mem::{size_of, align_of, swap}`, `core::intrinsics` (including
  the atomic operations `std::sync::atomic` is built on).
* `core::iter::Iterator`: `type Item`, `next`, and the adapters `map`,
  `filter`, `enumerate`, `take`, `skip` (lazy: they give new iterators,
  `core::iter::{Map, Filter, Enumerate, Take, Skip}`) and `count`. Every
  iterator of the standard library has them: `s.split(" ").map(|w|
  w.len()).filter(|n| n > 2).count()`.
* `core::marker::{Send, Sync, Detached}` and `core::cell::UnsafeCell` (§3).
* `core::ops::{Fn, FnMut, FnOnce}` (see Closures).
* `core::hash::{Hash, Hasher, hash_of}`: one fixed fast hasher, `Hash` for
  the primitives, `str`, `String`, tuples, `Option` and slices, and
  `#[derive(Hash)]`.
* `core::collections`: containers of Light items, like arrays, sharing one
  API: `insert`, `get` (a copy), `contains`, `remove`, `len`, `clear`,
  `iter`, and `update(k, |v| ..)` to change a value in place.
  * `HashMap` and `OrderedHashMap` (iterates in insertion order): keys
    `Hash + Eq`, with `insert` returning the old value, `get_or`, and
    `keys`/`values`.
  * `HashSet`: `insert`/`remove` return whether anything changed;
    `from_slice`, `extend`, `is_subset`, `is_disjoint`, `union`,
    `intersection`, `difference`.
  * `SortedMap`: keys `Ord`, kept in a sorted array (binary search;
    inserting moves later entries), iterated in order, with `range(lo,
    hi)`, `first`, `last`, `pop_first`, `pop_last`.
  * `Deque`: a growable ring buffer (`push_back`, `push_front`,
    `pop_back`, `pop_front`, `front`, `back`, `get`, `set`).
  * `BinaryHeap`: a max-heap (`push`, `pop`, `peek`, `from_slice`,
    `into_sorted`); push `cmp::Reverse(x)` to pop the smallest first.
* `core::arena::{Arena, Id, List}`: items of one type in one growable
  array, reached by typed handles (`add`, `get`, `set`, `update`, `view`,
  `view_mut`, `ids`); `add_list`/`list` store runs of items, such as a
  node's children. Trees are built from these.
* `core::intern::{Interner, Symbol}`: `intern(s)`, `get(s)`, `resolve(sym)`.
* Methods on slices (`len`, `get`, `first`, `last`, `contains`, `index_of`,
  `fill`, `swap`, `reverse`, `sort`, `to_dyn`, `to_grow`), growable arrays
  (`push`, `extend`, `reserve`), shrinkable arrays (`pop`, `truncate`,
  `clear`, `remove`, `insert`).
* `str`: `len`, `as_bytes`, `chars`, `char_indices`, `char_count`, `bytes`,
  `find`, `rfind`, `contains`, `starts_with`, `ends_with`, `strip_prefix`,
  `strip_suffix`, `trim` (also `trim_start`, `trim_end` and the `_matches`
  forms), `split`, `splitn`, `rsplit`, `rsplitn`, `split_once`,
  `rsplit_once`, `split_whitespace`, `split_at`, `lines`, `replace`,
  `replacen`, `repeat`, `to_lowercase`/`to_uppercase` (Unicode simple case
  mapping, one character for one, from the table in `core::unicode`) and
  the `to_ascii_` forms, `eq_ignore_ascii_case`, `parse::<T>()`,
  `to_string`, and `str::from_utf8(bytes): Option::<str>`. `char`:
  `to_lowercase`, `to_uppercase`, `is_lowercase`, `is_uppercase`, the
  `is_ascii_` classes, `to_digit`, `char::from_digit`.
* `String`: `new`, `from`, `from_utf8`, `from_utf8_lossy`, `push`,
  `push_str`, `write`, `as_str`, `len`, `clear`, `truncate`, `pop`,
  `insert`, `insert_str`, `remove`, `retain(|c| ..)`, `split_off`.
* Parsing: `core::str::FromStr` (`type Err; fn from_str(s: str):
  Result::<Self, Self::Err>`) for every integer and float type, `bool`,
  `char` and the `std::net` addresses, all with `Err = ParseError`;
  `s.parse::<T>()` calls it, giving `Result::<T, T::Err>`.
  `core::num::ParseError` says why (`kind()`: `Empty`, `InvalidDigit`,
  `PosOverflow`, `NegOverflow`, `Invalid`).
* Integer methods, on every width: `checked_`, `wrapping_` and
  `saturating_` `add`, `sub`, `mul`, `pow` (and checked `div`, `rem`,
  `neg`), `pow`, `abs`, `unsigned_abs`, `abs_diff`, `signum`,
  `div_euclid`/`rem_euclid`, `leading_zeros`, `trailing_zeros`,
  `count_ones`, `count_zeros`, `rotate_left`/`rotate_right`,
  `is_power_of_two`, `next_power_of_two`, `to_string`,
  `to_string_radix(r)` and `T::from_str_radix(s, r)`. `f64`: `sqrt`,
  `powf`, `powi`, `round`, `trunc`, `fract`, `floor`, `ceil`, `abs`, the
  trigonometric and logarithm functions, `is_nan`, `is_finite`, `min`,
  `max`, `clamp`, `to_string`.
* `std::io`:
  * `print`, `println`, `eprint`, `eprintln` (one write per call: lines
    from different calls never mix), `write_str`, and the unbuffered
    `read_line`, `read`, `read_exact`, `write_bytes`.
  * `Error` and `ErrorKind` for failures (`kind()`, `os_code()`,
    `Display`): `NotFound`, `PermissionDenied`, `AlreadyExists`,
    `WouldBlock`, `TimedOut`, `ConnectionRefused`, `BrokenPipe`,
    `UnexpectedEof`, `InvalidInput`, `InvalidData` and more.
  * The traits `Read` (`read` into a slice; `read_exact`, and
    `read_to_end`/`read_to_string` onto a `&mut +` buffer) and `Write`
    (`write`, `flush`; `write_all`, `write_str`, and `print(..)` /
    `println(..)` for format strings), implemented by `fs::File`, the
    standard streams, `net::TcpStream`, `Cursor` (reading a byte slice)
    and growable byte arrays (writing). Both can be used as trait objects:
    `fn emit(out: dyn mut Write)` takes any of them.
  * `stdin()` (buffered for the whole program: `read_line(&mut +String)`,
    `lines()`), `stdout()` and `stderr()`. `stdout().buffered()` is a
    `BufWriter` for printing much in a loop: one write per 8 KiB instead
    of one per line.
  * `BufReader` (`read_line` onto a `String`; `next_line()` and then
    `line()`, a `str` view into its buffer valid until the next read;
    `lines()`, an iterator of `String`s; `read_until`, `fill_buf`,
    `consume`) and `BufWriter` (`flush`, `into_inner`). A `BufWriter` over
    a file, standard stream or socket writes what it holds when it is
    freed; over anything else, `flush` it first. Both lend the stream
    inside to a closure with `with_inner(|s| ..)`.
* `std::fs`:
  * `File` (`open`, `create`, `create_new`, `options`; `Read`, `Write`,
    `seek(SeekFrom)`, `rewind`, `stream_position`, `sync_all`,
    `set_len`, `metadata`, `close`). A file is closed when it is freed:
    the runtime closes handles owned by an `rt::OwnedHandle`, and holding
    one makes a type move rather than copy.
  * `OpenOptions::new().read(true).append(true)...open(path)`, with
    `read`, `write`, `append`, `truncate`, `create`, `create_new`.
  * `metadata` and `symlink_metadata` give a `Metadata`: `len`,
    `is_file`, `is_dir`, `is_symlink`, `readonly`, `modified`,
    `accessed`, `created` (`SystemTime`s).
  * Whole files by path: `read_to_string` (`ErrorKind::InvalidData` if not
    UTF-8), `read`, `write`.
  * `copy`, `rename`, `remove_file`, `hard_link`, `symlink`, `symlink_dir`
    (Windows needs Developer Mode or administrator rights for links),
    `set_readonly`, `exists`, `is_file`, `is_dir`, `create_dir`,
    `create_dir_all`, `remove_dir`, `remove_dir_all`, `read_dir`,
    `canonicalize`. Paths are `str`s; everything returns
    `Result::<_, io::Error>`.
* `std::path`: `Path::new(s)`, a view of a path (`parent`, `file_name`,
  `file_stem`, `extension`, `components`, `is_absolute`, `has_root`,
  `starts_with`, `ends_with`, `strip_prefix`, `join`, `with_extension`,
  `with_file_name`, `normalize` (`.` and `..` from the text alone),
  `to_native`), and `PathBuf`, an owned path built with `push`, `pop`,
  `set_file_name`, `set_extension`. On Windows both separators and
  prefixes (`C:`, `\\server\share`, `\\?\`) are understood; `MAIN_SEPARATOR`
  is `\` there. `path::absolute(p)` joins to the current directory.
* `std::time`: `Duration` (`from_secs`, `from_millis`, `from_micros`,
  `from_nanos`, `from_secs_f64`, `as_secs`, `as_millis`, `subsec_nanos`,
  ..., `+`, `-`, `checked_`/`saturating_` arithmetic, `Display` such as
  `1.5s`), `Instant` (monotonic: `now`, `elapsed`, `duration_since`,
  `checked_add`), `SystemTime` (`now`, `duration_since`, `elapsed`,
  `time::unix_epoch()`), and `sleep(d)` (also `thread::sleep`).
* `std::thread` (§6a): `spawn(f)` and `Builder::new().name(..).stack_size(..)
  .spawn(f)` give a `JoinHandle::<T>` (`join`: `Result::<T, Panic>`,
  `is_finished`, `id`); `Panic` has the `message()` and the `location()`
  of a failed check. Arrays cannot hold handles (they own their thread's
  record), so `Group::<T>` keeps any number of threads (`spawn`, `len`,
  `join_next`). `scope(|s| ..)` with `Scope::spawn` and `spawn_with` for
  threads that borrow (`ScopedJoinHandle`). `sleep`, `yield_now`,
  `current_id()` (a `ThreadId`; the main thread's is 1), `current_name()`
  (`"main"` on the main thread) and `available_parallelism()`.
* `std::sync`: locks used through closures, all with `const fn new` (so
  they can be statics):
  * `Mutex::<T>`: `lock(|v| ..)` (panics if poisoned), `lock_checked`
    (`Err(PoisonError)` with the closure's result if poisoned), `try_lock`
    (`TryLockError::WouldBlock` or `Poisoned`), `lock_when(&cv, |v| ready,
    |v| ..)` waits on a `Condvar` until a condition holds, and
    `lock_when_timeout(&cv, d, ready, |v, ready| ..)`; `is_poisoned`,
    `clear_poison`, `get_mut`, `into_inner`.
  * `RwLock::<T>`: `read(|v| ..)` (many at once), `write(|v| ..)`, and the
    `_checked` and `try_` forms; writers go before new readers.
  * `Condvar`: `notify_one`, `notify_all` (waiting is `Mutex::lock_when`).
  * `Once` (`call_once`, `is_completed`), `OnceLock::<T>` (`set`, `with`,
    `with_or_init`, and for Light values `get`, `get_or_init`) and
    `LazyLock::<T>` (`LazyLock::new(f)` with a function `f`; `with`, `get`).
  * `Barrier::new(n)` (`wait`; true for one leader).
  * `Shared::<T>`: a value with several owners across threads, freed with
    the last (`new`, `clone`, `with(|v| ..)`, `count`, `same`,
    `try_unwrap`); `T` must be `Send + Sync + Detached`.
  * `atomic`: `AtomicBool`, `AtomicU8`..`AtomicU64`, `AtomicUsize`,
    `AtomicI8`..`AtomicI64`, `AtomicIsize` (all `Atomic::<T>`) and
    `AtomicPtr::<T>`, with `load`, `store`, `swap`, `compare_exchange`
    (`_weak`), `fetch_add`, `fetch_sub`, `fetch_and`, `fetch_or`,
    `fetch_xor`, `fetch_nand`, `fetch_max`, `fetch_min`, `fetch_update`,
    `into_inner`; `Ordering` (`Relaxed`, `Release`, `Acquire`, `AcqRel`,
    `SeqCst`, as in C++) and `fence(order)`.
  * `mpsc::channel()` (unbounded) and `mpsc::sync_channel(n)` (bounded; 0
    is a rendezvous): `Sender` (`Clone`), `SyncSender` (`send` waits for
    room, `try_send`), `Receiver` (`recv`, `try_recv`, `recv_timeout`, and
    iteration until every sender is gone). `mpmc::channel()` and
    `mpmc::bounded(n)` with both ends `Clone` and `send_timeout`. Sending
    fails, giving the value back, once every receiver is gone. Messages are
    `Send + Detached`.
  * `raw`: `RawMutex`, `RawCondvar` (on the system's futex) and `Queue::<T>`,
    a first-in first-out queue of any values, owners included.
* `std::random`: `fill(buf)` and `entropy_u64()` from the operating
  system's secure source, and `Rng`, a xoshiro256** generator
  (`Rng::new()` seeded from the system, `Rng::seeded(n)` repeatable):
  `next_u64`, `below(n)`, `range(lo, hi)` and `range_inclusive` for every
  number type, `bool`, `chance(p)`, `f64`, `shuffle(slice)`,
  `choose(slice)`.
* `std::net`: `Ipv4Addr`, `Ipv6Addr`, `IpAddr` and `SocketAddr` (parsed
  with `FromStr`, displayed per RFC 5952); `lookup_host(name)` and
  `resolve("host:port")`; `TcpListener` (`bind`, `accept`, `local_addr`),
  `TcpStream` (`connect`, `connect_timeout`, `Read`, `Write`, `peek`,
  `shutdown`, `set_nodelay`, read and write timeouts) and `UdpSocket`
  (`send_to`, `recv_from`, `connect`, `send`, `recv`). I/O blocks, or
  fails with `WouldBlock` after `set_nonblocking(true)`. Sockets are
  closed when freed.
* `std::math` (`pow`, `min`, `max`, `clamp`, float functions),
  `std::test::{assert, assert_eq, assert_ne}` (functions taking `&T`, and
  the macros `assert!(c)`, `assert_eq!(a, b)`, `assert_ne!(a, b)`, which
  lend their arguments and describe the assertion),
  `std::process::{exit, abort, panic}`, `std::env::{arg_count, arg, args,
  var, vars}` (`args()` iterates `str`s; `vars()` is a snapshot whose
  `iter()` gives `(name, value)` views), and
  the macro `std::env::env!("NAME")`: a `str` literal with the value the
  variable had when the program was compiled. `LUNAR_PKG_NAME` and
  `LUNAR_PKG_VERSION` are the name and version of the package being built
  (from `lunar.toml`); other names come from the compiler's environment.
  An undefined variable is `err(macro::error)`.
* The prelude provides `Dull`, `Light`, `Send`, `Sync`, `Detached`,
  `Option`/`Some`/`None`, `Result`/`Ok`/`Err`,
  `Eq`, `Ord`, `Ordering`, `Clone`, `Default`, `Display`, `Debug`, `Hash`,
  `Iterator`, `Fn`, `FnMut`, `FnOnce`, `print`, `println`, `eprint`,
  `eprintln`, `panic`, `assert`,
  `assert_eq` and `assert_ne`, with the derives and macros of the same
  names, and the macro `env!`.
* Format strings: `{x}` uses `Display`, `{x:?}` uses `Debug` (strings and
  characters quoted and escaped, arrays as `[1, 2]`, tuples as `(1, 2)`,
  derived types as `Point { x: 1, y: 2 }`), and format specs such as
  `{x:>8.2}` (see [Format specs](#format-specs)). Fixed arrays (`[T; 4]`)
  implement `Debug` when their items do, built in (there are no const
  generics for an impl in `core`), the way their slices show: `[1, 2, 3]`.
* Derives in `core`: `#[derive(Eq, Ord, Debug, Clone, Default, Hash)]` on structs
  and enums (`Default` uses the variant marked `#[default]`), and
  `#[derive(Display)]` on enums whose variants have no fields. They are
  ordinary macros (`core/cmp.ln`, `core/fmt.ln`, ...), not built into the
  compiler.
