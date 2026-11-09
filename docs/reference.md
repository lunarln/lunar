# The Lunar Language Reference

## 1. Lexical structure

* Comments: `// line`, `/* block */` (block comments nest).
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
| `!T` | `&mut` parameters and receivers only, where `T` contains something shrinkable (`[U; -..+]`, `String`, or a type containing one): the argument may be shrunk (see §4) |
| `str` | UTF-8 view (like Rust's `&str`) |
| `String` | owned UTF-8 text (a `[u8; -..+]` with a UTF-8 invariant) |
| `FmtArgs` | parameter only: an interpolated string literal |
| `*T`, `*mut T` | raw pointers (dereferencing is unsafe) |
| `fn(A, &B, &mut C): R` | function pointers; parameter modes are part of the type |
| `!` | the never type (return type of functions that never return) |

### Dull and pointy types

`core::ptr::Dull` is implemented **automatically by the compiler** for every
type that is not a pointer, slice, string or array and contains none. Such
types are *Dull*; all others are *pointy*. `Dull` cannot be implemented or
`!`implemented manually (`err(trait::dull_impl)`).

**Array and slice items must be Dull:**

```text
err(type::array_pointy_items): Arrays and slices must have an item type that is not pointy.
help: add Dull to T's bounds: `T: Dull`
```

Dull values are copied implicitly. Pointy views (slices, `str`) are copied
too. Owning values (owning arrays, `String`, and types containing them) are
**moved**.

## 4. Memory model: data stacks

Every owning array (`[T]`, `[T; ..+]`, `[T; -..+]`, `String`) is a small
handle `{ ptr, len, cap }` pointing at its own *data stack*: a 1 GiB slot of
address space whose pages are committed as the array grows. An array can
hold at most 1 GiB; growing past that is a fatal `data stack overflow`.

All slots come from one region reserved at startup (up to 64 TiB, so up to
65,536 arrays can be alive at once). Taking and returning a slot never calls
the operating system. On Linux and macOS the region is usable up front and
the kernel supplies pages on first touch, so allocation makes no system
calls after startup; on Windows, memory is committed only when a slot grows
past the most it has ever held. Freed slots keep their memory for reuse, up
to 64 MiB in total.

* **Growing never moves elements.** Slices taken from a `[T; ..+]` stay
  valid after any number of `push`es.
* **Shrinking can invalidate slices.** A function that may shrink its
  argument takes it as `!T`, where `T` is `[U; -..+]`, `String`, or any
  struct, enum or tuple containing one at any depth:

  ```lunar
  fn clear_all(a: &mut ![i32; -..+]) { ... }
  fn drain(s: &mut !Stack): i32 { ... }      // struct Stack { items: [i32; -..+] }

  impl Stack {
      fn pop(&mut !self): Option::<i32> { ... }   // a shrinking method
  }
  ```

  Methods that may shrink their receiver declare it `&mut !self`, as
  `pop`, `truncate` and `clear` on `[T; -..+]` and `String` do. Calling
  such a method follows the same rules as passing the receiver to a `!`
  parameter.
* `!` always comes with `&mut`: shrinking happens in place, so a `!`
  parameter that is not in-out is `err(type::bang_without_mut)`. A `!` on a
  type with nothing shrinkable in it is `err(type::bang_not_shrinkable)`.
* Passing a value to a `!` parameter (or calling a `&mut !self` method on
  it) is only allowed:
  * for the value's **owner** (a local variable, a parameter that took
    ownership, or a parameter that is itself declared `!`):
    `err(borrow::shrink_not_owner)` otherwise;
  * when **no live view** points into the value: the compiler's liveness
    analysis reports `err(borrow::shrink_while_borrowed)` if a view is used
    after the shrink.
* A `!T` can be passed anywhere a plain `T` is expected.
* Views are tracked per variable, not per field: a view into any part of a
  value blocks shrinking the whole value, even a disjoint field (a slice of
  `stack.name` blocks `drain(stack)`). This is conservative but sound;
  tracking disjoint fields separately is planned.
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
| `x: &mut !T`, `&mut !self` | in-out and may be shrunk (see above) |

Call sites look the same whatever the modes: `f(name)` moves, borrows or
passes in-out depending on `f`'s signature, and using a moved variable
afterwards is `err(borrow::use_after_move)`.

```lunar
fn new(name: String, age: u32): Person {   // keeps `name` without copying it
    return Person { name, age };
}
fn len(s: &String): usize { return s.len(); }
fn bump(x: &mut i32) { x += 1; }

let mut n = 1;
bump(n); // n == 2
```

`&T` and `&mut T` exist only on parameters; they are modes, not types you
can store. `str` and `[T; &]` are the first-class views. Borrowing a Dull
type is pointless (`warn(lint::needless_borrow)`), and a `mut` parameter
that is changed but never read again probably meant `&mut`
(`warn(lint::mut_param_not_read)`).

Borrowed parameters are passed by address, so a view into a borrowed value
points into the caller's data and may be returned:
`fn first_two(a: &[i32; 4]): [i32; &] { return a[0..2]; }`.

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
let n = text.find("x")?; // `?` on Option and Result
```

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

### Format strings

A string literal is interpolated only when it is passed to a `FmtArgs`
parameter, such as `println`, `print`, `panic`, `String::write` or
`core::fmt::format`. `{expr}` formats `expr` with `Display`; `{{` and `}}`
are literal braces. Everywhere else a string literal is a plain `str`.

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
  `needless_borrow` and `unknown_cfg`. The first four are reported only
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
  `core::fmt::{Display, write, format}`, `core::option::Option`,
  `core::result::Result`, `core::iter::Iterator`, `core::num::{Int, Float}`,
  `core::mem::{size_of, align_of, swap}`, `core::intrinsics`.
* Methods on slices (`len`, `get`, `first`, `last`, `contains`, `index_of`,
  `fill`, `swap`, `reverse`, `sort`, `to_dyn`, `to_grow`), growable arrays
  (`push`, `extend`, `reserve`), shrinkable arrays (`pop`, `truncate`,
  `clear`, `remove`, `insert`), `str` (`len`, `as_bytes`, `chars`, `bytes`,
  `find`, `contains`, `starts_with`, `ends_with`, `trim`, `parse_i64`,
  `to_string`) and `String` (`new`, `from`, `push`, `push_str`, `write`,
  `as_str`, `len`, `clear`, `truncate`, `pop`).
* `std::io::{print, println, eprint, eprintln, write_str, read_line}`,
  `std::math` (`pow`, `min`, `max`, `clamp`, float functions),
  `std::test::{assert, assert_eq, assert_ne}`,
  `std::process::{exit, abort, panic}`, `std::env::{arg_count, arg}`.
* The prelude provides `Dull`, `Option`/`Some`/`None`, `Result`/`Ok`/`Err`,
  `Eq`, `Ord`, `Ordering`, `Clone`, `Default`, `Display`, `Iterator`,
  `print`, `println`, `eprint`, `eprintln`, `panic`, `assert` and
  `assert_eq`.
