# Lunar style

The style `lunar fmt` writes (`craters/ln-fmt`). There is one, with no
options, like gofmt's. It is rustfmt's default style, since Lunar's syntax
is Rust's for the most part, with rules of its own for what Lunar adds.

```lunar
use crater::source::{NONE, Source, newlines, scan};

/// What the layout keeps of a line.
pub struct Out {
    text: String,
    pos: i64,
}

impl Out {
    pub fn print(&mut +self, src: str, ind: u32, s: str): str from src {
        if self.pending && s.len() > 0 {
            self.newline(src, ind);
        }
        let kind = if comment { Kind::Comment } else { Kind::Code };
        match kind {
            Kind::Comment => self.suffix.push(Span { lo, hi }),
            Kind::Code => {
                self.text.push_str(s);
                self.pos += measure(s);
            }
        }
        return src[..self.pos as usize];
    }
}
```

## Lines

* 4 spaces of indentation, lines of at most 100 columns.
* No trailing whitespace; one line break at the end of the file. A file
  whose first line ends with `\r\n` keeps `\r\n` line endings.
* Between items, statements, fields, variants and match arms, an empty line
  stays where the source has one (several become one); none is added. None
  after a `{` or before a `}`.
* What does not fit is broken at its outermost level first: an argument
  list before the arguments in it.
* What cannot be broken stays long: string literals, long paths, comments.

## Spaces

* One space around binary operators, `=` and `=>`, after `,` and `:`
  (`x: i32`), none before them; none inside brackets: `f(a, b)`, `[1, 2]`.
* `{ a, b }` with spaces for struct literals, patterns and variants;
  `{a, b}` in `use` lists.
* No space in paths and generic arguments: `Pair::<i32>::new`,
  `size_of::<T>()`.
* Ranges without spaces: `0..n`, `a..=b`, `..`. Unary operators without a
  space: `-x`, `!x`, `*p`, `&x`, `&mut x`, `& &x` (`&&` would be one
  token).

## Lists

Argument lists, parameters, generic parameters, tuples, arrays, struct
literals and patterns are written on one line if they fit, else one item a
line, indented, with a trailing comma:

```lunar
fn long_function_name(
    first_argument: SomeLongType,
    second_argument: AnotherLongType,
): ReturnType {
    call_something(
        first_argument_value,
        second_argument_value,
        third_value_xyz1,
    );
}
```

rustfmt's width heuristics decide what counts as fitting, besides the 100
columns: the arguments of a call at most 60 columns wide (a single argument
may take the whole line), array items 60, struct literal and struct pattern
fields 18, a struct variant's fields 35, a method chain 60, a one-line
`if ... else` 50, a one-line `let ... else` 50.

* Struct and enum declarations have a field or variant a line, always
  (`Rect { w: f64, h: f64 },` stays on its line as a short variant).
* Arrays of short items (literals and paths of at most 10 columns) are
  filled, as many to a line as fit; so are `use` lists without nested
  lists.
* A call's last argument may hug the parentheses instead of the arguments
  breaking: a closure or block always (`items.map(|x| {` ... `})`), and, as
  the only argument, a struct literal, array, call, `match` or `if`
  (`v.push(Point {` ... `})`).
* Types do not break, nor generic argument lists: a signature too long for
  its line breaks its parameters.

## Expressions

* A chain of one operator breaks before the operator, the operands after
  the first indented. The condition of an `if`, `while` or `match` breaks
  with its `{`, which then starts a line of its own:

  ```lunar
  if first_condition_is_long(a)
      && second_condition_is_longer(b)
  {
  ```
* What follows `=` (in `let`, assignments, `const`, `static`, struct
  literal fields after `:`, match arms after `=>`) stays after it if it
  fits, else goes on the next line, indented, if it fits there whole, else
  stays and breaks.
* A method chain wider than 60 columns breaks before each `.`. A root as
  short as an indentation (`self`, `out`) keeps its first link:
  `self.items` then `.iter()`. If the root itself breaks (`foo(` ... `)`),
  the links follow at its indentation. `f(x).y()` is a chain; `a.b()` is
  not.
* Blocks have a statement a line. A block of just an expression may be one
  line where it is an expression (`unsafe { f() }`, `|x| { x }`); function
  and loop bodies never are, except `{}`.
* `if c { a } else { b }` stays on one line if it is at most 50 columns;
  otherwise `} else {` and `} else if c {`.
* Match arms: `pat => expr,`; no comma after a block body. A long arm body
  goes on the next line, indented. Alternatives `A | B` break before `|`.
* Parentheses, braces, `;` and `,` are never added or removed where that
  would change the syntax tree: rustfmt's adding braces around long match
  arms and removing them around closure bodies are not done.

## Items

* Attributes one a line, before the item, as written.
* `pub`, `unsafe`, `fn`, then the name, generics `::<T: A + B, U>`,
  parameters, `: ReturnType`, `from a, b`, and the body.
* `use` lists are sorted as rustfmt sorts them: `self`, `super` and
  `crater` first, then by "version sorting" (byte by byte, so uppercase
  before lowercase, with runs of digits compared as numbers: `a9` before
  `a10`). A list with comments in it keeps its order. Consecutive `use`
  items keep theirs.

## What Lunar adds

* `::<>` for generic arguments and parameters, everywhere, without spaces.
* Parameter modes as in signatures: `x: T`, `x: &T`, `x: &mut T`,
  `x: &mut +T`; receivers `self`, `mut self`, `&self`, `&mut self`,
  `&mut +self`; `kept` first: `kept mut x: &T`.
* Return types after `:`, and their `from` clause after them:
  `fn first(a: &String, b: &String): str from a, b`.
* Array types `[T; 4]`, `[T; _]`, `[T]`, `[T; ..+]`, `[T; -..+]`, `[T; &]`,
  `[T; &mut]`; `+T`; pointers `*T`, `*mut T`; function types with modes
  `fn(A, &B, &mut C): R`; bounds `Fn(&String): usize`; `impl Fn(i32): i32`;
  closures with a return type `|x: i32|: i32 { x }`.
* String and character literals are never changed, nor the `{...}` holes
  of format strings. Number literals keep their form (`0xff_u8`, `1.`).
* Macro calls whose arguments are expressions are formatted as calls
  (`assert_eq!(a, b)`), keeping a trailing comma exactly where the source
  has one (it is part of the macro's input). Other macro calls, and
  `macro` definitions, keep their layout; their lines move with the line
  they start on (lines inside multi-line strings and comments never move).

## Comments

Comments are kept, in order, where they are among the code:

* A comment at the end of a line stays at the end of that line, after the
  `,` or `;` there. A list holding one is broken, one item a line.
* A comment on a line of its own stays on a line of its own, indented as the
  code after it. Empty lines around comments between items and statements
  are kept as other empty lines are.
* Doc comments (`///`) and `//!` comments are comments like the others.
  Block comments keep their text.
* Trailing whitespace is removed from line comments; comments are not
  wrapped or reflowed.

## Safety

`lunar fmt` never changes what a program means. Each result is parsed again
and its syntax tree compared with the original's (without the positions,
and with `use` lists in any order), and its comments with the original's;
on any difference the file is left as it is and the difference reported as
a bug. A file with syntax errors is left as it is, and its errors shown.
Formatting a formatted file changes nothing.
