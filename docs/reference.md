# The Lunar Language Reference

## 1. Lexical structure

* Comments: `// line`, `/* block */` (block comments nest).
* Doc comments: `/// text` (exactly three slashes) documents the item,
  enum variant or struct field that follows; it is the attribute
  `#[doc = "text"]`, one per line. Elsewhere it is an ordinary comment.
* Identifiers: `[a-zA-Z_][a-zA-Z0-9_]*`. `_` alone is the placeholder.
* Keywords: `as break const continue crater else enum extern false fn for if
  impl in let loop match mod mut pub return self Self static struct super
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

use std::io::{print, println as say};
use crater::shapes::*;
mod shapes;            // loads shapes.ln or shapes/mod.ln
mod inline { ... }
const N: usize = 4;
static mut COUNT: u64 = 0;
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

### Attributes

* `#[test]`: a test function, run by `lunar test`.
* `#[cfg(windows)]`, `#[cfg(unix)]`, `#[cfg(target_os = "linux")]`,
  `#[cfg(target_arch = "aarch64")]`, `#[cfg(test)]`, `not(...)`, `all(...)`,
  `any(...)`: conditional compilation. Any other name or `target_os` value
  is never set, so it is always false; the compiler warns about it
  (`warn(lint::unknown_cfg)`), e.g. `#[cfg(linux)]` for
  `#[cfg(target_os = "linux")]`.
* `#[link_name = "..."]` on extern functions.
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
  Light (`Dull: Light`), and so are views (`str`, `[T; &]`, raw pointers)
  and anything built only from Light types. `String`, owning arrays and
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
  `String::write`, `Arena::add` and `Interner::intern` are `+`. Methods
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
for c in s.chars() { }   // anything implementing Iterator::<T>
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
  `char`, and between pointers and `usize`.
* Operators on user types go through the traits in `core::ops` and
  `core::cmp` (`Add`, `Sub`, ..., `Eq` for `==`, `Ord` for `<`).
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
three.

A function can return a closure (or any value) without naming its type:
`fn make_adder(n: i32): impl Fn(i32): i32 { return move |x| x + n; }`.
Callers see only the bounds after `impl`; the body returns one concrete
type. A returned closure must be `move`.

### Format strings

A string literal is interpolated only when it is passed to a `FmtArgs`
parameter, such as `println`, `print`, `panic`, `String::write` or
`core::fmt::format`. `{expr}` formats `expr` with `Display`; `{{` and `}}`
are literal braces. Everywhere else a string literal is a plain `str`.

A hole may contain string literals: `println("{m.get("k")} {greet("moon")}")`.
Because any string literal may turn out to be a format string, the lexer
reads a `{` as opening a hole only when what follows looks like an
expression: brackets balance up to a matching `}`, and each `"` inside
comes where an operand can begin (after an operator, `(`, `[`, `{` or `,`,
not first in the hole) and closes on the same line. Otherwise the `{` is
text, so plain strings such as `"{"`, `"} else {"` and `"{\"k\": 1}"` mean
what they always did.

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

## 7. Traits

Traits work like Rust's: `trait Name::<Params>: Supertraits { fn m(self); }`
with default methods, static functions (`T::default()`), generic and blanket
impls (`impl::<T: Dull> Clone for [T; ..+]`), and negative impls only for
auto traits (and `Dull` cannot be negated). Trait method calls are resolved
statically and monomorphized; there are no trait objects.

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

* `core::ptr::Dull`, `core::ops`, `core::cmp::{Eq, Ord, Ordering, min, max}`,
  `core::clone::Clone`, `core::default::Default`,
  `core::fmt::{Display, Debug, write, format}`, `core::option::Option`,
  `core::result::Result`, `core::iter::Iterator`, `core::num::{Int, Float}`,
  `core::mem::{size_of, align_of, swap}`, `core::intrinsics`.
* `core::ops::{Fn, FnMut, FnOnce}` (see Closures).
* `core::hash::{Hash, Hasher, hash_of}`: one fixed fast hasher, `Hash` for
  the primitives, `str`, `String`, tuples, `Option` and slices, and
  `#[derive(Hash)]`.
* `core::collections::{HashMap, OrderedHashMap}`: maps from Dull keys
  (`Hash + Eq`) to Dull values, with `insert` (returns the old value),
  `get` (a copy), `get_or`, `contains`, `remove`, `len`, `update(k, |v| ..)`
  and `iter`/`keys`/`values`. `OrderedHashMap` iterates in insertion order.
* `core::arena::{Arena, Id, List}`: items of one type in one growable
  array, reached by typed handles (`add`, `get`, `set`, `update`, `view`,
  `view_mut`, `ids`); `add_list`/`list` store runs of items, such as a
  node's children. Trees are built from these.
* `core::intern::{Interner, Symbol}`: `intern(s)`, `get(s)`, `resolve(sym)`.
* Methods on slices (`len`, `get`, `first`, `last`, `contains`, `index_of`,
  `fill`, `swap`, `reverse`, `sort`, `to_dyn`, `to_grow`), growable arrays
  (`push`, `extend`, `reserve`), shrinkable arrays (`pop`, `truncate`,
  `clear`, `remove`, `insert`), `str` (`len`, `as_bytes`, `chars`, `bytes`,
  `find`, `contains`, `starts_with`, `ends_with`, `trim`, `split`, `lines`,
  `parse_i64`,
  `to_string`, and `str::from_utf8(bytes): Option::<str>`) and `String` (`new`, `from`, `push`, `push_str`, `write`,
  `as_str`, `len`, `clear`, `truncate`, `pop`).
* `std::io::{print, println, eprint, eprintln, write_str, read_line}`, and
  `std::io::{Error, ErrorKind}` for failures (`kind()`, `os_code()`,
  `Display`),
* `std::fs::{read_to_string, read, write, remove_file}`: whole files by
  path, returning `Result::<_, io::Error>`; `read_to_string` fails with
  `ErrorKind::InvalidData` on text that is not UTF-8,
  `std::math` (`pow`, `min`, `max`, `clamp`, float functions),
  `std::test::{assert, assert_eq, assert_ne}` (functions taking `&T`, and
  the macros `assert!(c)`, `assert_eq!(a, b)`, `assert_ne!(a, b)`, which
  lend their arguments and describe the assertion),
  `std::process::{exit, abort, panic}`, `std::env::{arg_count, arg}`.
* The prelude provides `Dull`, `Option`/`Some`/`None`, `Result`/`Ok`/`Err`,
  `Eq`, `Ord`, `Ordering`, `Clone`, `Default`, `Display`, `Debug`, `Hash`,
  `Iterator`, `Fn`, `FnMut`, `FnOnce`, `print`, `println`, `eprint`,
  `eprintln`, `panic`, `assert`,
  `assert_eq` and `assert_ne`, with the derives and macros of the same
  names.
* Format strings: `{x}` uses `Display`, `{x:?}` uses `Debug` (strings and
  characters quoted and escaped, arrays as `[1, 2]`, derived types as
  `Point { x: 1, y: 2 }`).
* Derives in `core`: `#[derive(Eq, Ord, Debug, Clone, Default, Hash)]` on structs
  and enums (`Default` uses the variant marked `#[default]`), and
  `#[derive(Display)]` on enums whose variants have no fields. They are
  ordinary macros (`core/cmp.ln`, `core/fmt.ln`, ...), not built into the
  compiler.
