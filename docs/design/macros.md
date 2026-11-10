# Macros and derive

Status: decided and implemented 2026-11-09. Not yet supported: items in a
macro expanded in expression position (`macro::not_supported`), and paths
inside `#[derive(...)]` (name derives you have imported: `#[derive(Eq)]`).
Derived `Eq`, `Ord`, `Debug` and `Clone` on enums bind each variant's
fields, so those fields must be `Dull` (as for `core`'s own `Option`).

The goal is that **no trait is derived by the compiler**. `Eq`, `Debug`,
`Display`, `Clone`, `Default` and `Ord` are derived by ordinary macros
written in `core`, using a macro system that any crater can use the same
way. The compiler provides the macro engine, never the derives.

The model is Rust's "macros 2.0" (declarative, hygienic, first-class items),
not `macro_rules!`.

## 1. Defining macros

A macro is an item, with visibility like any other:

```lunar
/// The square of an expression, evaluated once.
pub macro square($x:expr) {
    let t = $x;
    t * t
}

/// Several rules, tried in order; the first whose matcher accepts the input wins.
pub macro max {
    ($x:expr) => { $x },
    ($x:expr, $($rest:expr),+) => {
        {
            let a = $x;
            let b = max!($($rest),+);
            if a > b { a } else { b }
        }
    },
}
```

The one-rule form `macro name(matcher) { body }` is shorthand for
`macro name { (matcher) => { body } }`.

Macros are order-independent like other items: a macro can be used before
its definition in the same module, imported with `use`, and exported from a
crater with `pub`.

### Matchers

`$name:fragment` matches one syntactic fragment; anything else in a matcher
matches that token literally. Fragments, as in Rust:

| Fragment | Matches |
|---|---|
| `ident` | an identifier (not a keyword) |
| `path` | a path: `a::b::<T>` |
| `expr` | an expression |
| `ty` | a type |
| `pat` | a pattern (including `|` alternatives) |
| `block` | a `{ ... }` block |
| `stmt` | a statement without its trailing `;` |
| `item` | an item, with its attributes |
| `lit` | a literal, optionally negated |
| `vis` | `pub` or nothing |
| `meta` | the inside of an attribute: `name`, `name = "x"`, `name(...)` |
| `tt` | one token tree: a token, or a `()`/`[]`/`{}` group |
| `generics` | Lunar-specific: a generic parameter list `::<T: A + B, U>`, or nothing (see section 6) |

Repetitions are `$( ... ) sep op`, where `op` is `*` (zero or more), `+`
(one or more) or `?` (zero or one), and `sep` is an optional separator
token. Follow-set rules are Rust's: a fragment that can grow (`expr`, `ty`,
`path`, `pat`, ...) may only be followed by tokens that cannot continue it,
so matching never needs backtracking. A matcher that breaks this is an
error at the definition.

### Transcribers

The body is token trees. `$name` inserts what the fragment matched,
`$( ... ) sep op` repeats over the fragments used inside it (they must have
been matched at the same repetition depth), and everything else is copied.
The result is parsed in the position of the invocation (section 2).

## 2. Invoking macros

`path!(...)`, `path![...]` or `path!{...}`. The delimiter does not change
the meaning. In item and statement position, `()` and `[]` invocations end
with `;`, and `{}` invocations do not.

Macros can be invoked in five positions, and the expansion is parsed as
that kind of syntax:

```lunar
let v = vec![1, 2, 3];                  // expression
assert_eq!(v.len(), 3);                 // statement
tokens! { LParen "(", RParen ")" }      // item (also inside impl and trait bodies)
match c { digit!() => 1, _ => 0 }       // pattern
fn f(x: Pair![i32, str]) {}             // type
```

`name!` is unambiguous: a `!` directly after a path and before a delimiter
cannot be the prefix `!` (not, never) or `!=`.

## 3. Names

Macros live in a third namespace next to types and values. A path can name
a trait and a macro at once, and `use` imports every namespace a name
exists in:

```lunar
// core/cmp.ln
pub trait Eq { fn eq(&self, other: &Self): bool; }

#[derive_macro]
pub macro Eq { ... }

// anywhere: the prelude exports both
#[derive(Eq)]
struct Point { x: i32, y: i32 }
```

## 4. Hygiene

Hygiene is **def-site for every name**. A name written in a macro body
resolves where the macro is defined, whatever is in scope where it is used:

* `let` bindings in the body cannot clash with or capture the caller's
  locals (`let t = $x;` in `square!` cannot be confused with a caller's `t`);
* paths in the body (`Eq`, `core::fmt::write`, `bool`) resolve in the
  macro's defining module, even if the caller shadows them, so no `$crate`
  is needed;
* items the body defines under names written in the body are not
  reachable from the caller, unless the item is marked `#[export]`. To
  create an item the caller can name, either mark it `#[export]` or take
  its name as a parameter (`$name:ident`):

```lunar
pub macro tokens {
    ($($name:ident $text:lit),*) => {
        #[export]
        pub enum Token { $($name),* }   // the caller can use `Token`

        fn text(t: Token): str { ... }  // private to the expansion
    },
}

tokens! { LParen "(", RParen ")" }
let t = Token::LParen;   // fine
// text(t)               // error: `text` is not visible here
```

  `#[export]` makes the item's own name resolve at the call site; names
  inside the item (its fields, variants, and the paths in its body) keep
  their def-site meaning.

Names that come in through `$parameters` keep the caller's meaning.

Mechanism: every identifier carries a syntax context. Expanding a macro
gives the tokens copied from its body a fresh mark, tied to the macro's
definition. Tokens substituted from `$parameters` keep the context they had
at the call. Name resolution looks a marked name up in the definition's
scope, and local bindings only match names with the same context.

## 5. Derive macros

A derive is a macro marked `#[derive_macro]`. Writing `#[derive(A, B)]` on
a struct or enum invokes `A!` and then `B!` with the item as input, that
is, its tokens including its other attributes and doc comments but without
the `#[derive(...)]` itself. A derive can only **add** items: its expansion
must parse as items, which are placed after the deriving item in the same
module. The item itself is never changed.

```lunar
#[derive_macro]
pub macro Eq {
    ($(#[$m:meta])* $v:vis struct $name:ident $g:generics {
        $($(#[$fm:meta])* $fv:vis $f:ident: $t:ty),* $(,)?
    }) => {
        impl $g.bound(Eq) Eq for $name $g.args {
            fn eq(&self, other: &Self): bool {
                return true $(&& self.$f == other.$f)*;
            }
        }
    },
    ($(#[$m:meta])* $v:vis enum $name:ident $g:generics { $($variants:tt)* }) => {
        ...
    },
}
```

## 6. The `generics` fragment

Every derive has to copy the type's generic parameters into its impl and
add its own bound: `impl::<T: Dull + Eq> Eq for Pair::<T>`. Rust's
declarative macros cannot do that cleanly; Rust's real derives are
procedural. Lunar adds one fragment for it. `$g:generics` matches a
parameter list `::<T: A + B, U>` or nothing, and has projections:

| Written | Expands to (for `::<T: Dull, U>`) | Expands to (for nothing) |
|---|---|---|
| `$g` | `::<T: Dull, U>` | nothing |
| `$g.args` | `::<T, U>` | nothing |
| `$g.bound(Eq)` | `::<T: Dull + Eq, U: Eq>` | nothing |

## 6b. Metavariable expressions

As in Rust, `${...}` in a body computes tokens instead of copying them.
Derives need them to name and count fields:

| Expression | Expands to |
|---|---|
| `${index()}` | the index of the innermost repetition, as an integer literal (`self.${index()}` is `self.0`, `self.1`, ...) |
| `${ignore($x)}` | nothing; a repetition that uses it repeats once per match of `$x` |
| `${concat(a, index())}` | one identifier made of identifiers, literals, `$x` (an `ident` or `lit`) and `index()`: `a0`, `a1` |
| `${stringify(...)}` | a string literal of the tokens: `${stringify($n)}` is `"Point"` |
| `${error("message")}` | nothing; the expansion fails with `macro::error` and the message |

Identifiers made by `${concat(...)}` belong to the expansion (they are
hygienic like the rest of the body).

## 7. Debug and `{x:?}`

```lunar
// core/fmt.ln
pub trait Debug {
    fn debug(&self, out: &mut String);
}
```

A format hole `{expr:?}` calls `Debug::debug`; `{expr}` still calls
`Display::fmt`. The hole is split at its first top-level single `:` (not
inside brackets, braces or parentheses, and not part of `::`). No
expression can contain a top-level `:`, because Lunar has no ternary and no
type ascription, so `{p:?}`, `{a::b:?}` and `{Point { x: 1 }:?}` are all
unambiguous. `?` is the only format spec for now.

Core implements Debug for the primitives (numbers and `bool` as Display
does, `char` as `'a'`, `str` and `String` quoted and escaped as `"a\n"`),
for slices and arrays (`[1, 2]`), and for `Option` and `Result`.

## 8. The derives in core

| Derive | Structs | Enums |
|---|---|---|
| `Eq` | field by field | same variant and equal fields |
| `Debug` | `Point { x: 1, y: 2 }`, `Pair(1, 2)`, `Unit` | `Number(Int)`, `Line { from: 1, to: 2 }`, `Dot` |
| `Display` | error: write it yourself | **fieldless enums only**: the variant name; otherwise an error |
| `Clone` | field by field | variant by variant |
| `Default` | every field's default | the variant marked `#[default]` (fieldless) |
| `Ord` | lexicographic by field order | by variant order, then by fields |

Derived `Display` exists so that fieldless enums such as the lexer's
`Token` stop needing a hand-written 90-arm `name()`. Any other type's
`Display` is a design decision, so deriving it is an error that says to
write the impl.

## 9. What the compiler still knows

The macro engine, the `#[derive]` and `#[derive_macro]` attributes, the
`generics` fragment, and the existing lang items that connect syntax to
traits: `==` to `Eq` and `{x}` to `Display`, plus `{x:?}` to the new
`Debug` lang item. It knows nothing about how any trait is derived.

## 10. Diagnostics

New `macro::` codes: `undefined` (no macro by that name), `no_match` (no
rule matches the input; the note shows the closest rule and where it
stopped), `bad_matcher` (an unknown fragment, or a follow-set violation, at
the definition), `repetition` (fragments used at different repetition
depths), `recursion_limit` (expansion nested more than 128 deep), and
`derive_output` (a derive expanded to something other than items). Errors
inside expanded code also point at the invocation: "in this expansion of
`vec!`".

## 11. Build order

1. Explicit `&` / `&mut` at call sites (see the reference, Parameters).
2. Macro items, token trees, the matcher and the transcriber.
3. Hygiene and the macro namespace.
4. Expansion in expression, statement, item, pattern and type position.
5. `Debug`, `{x:?}`, `#[derive]` and the six derives in core.
6. The Lunar lexer in `craters/` derives `Eq`, `Debug` and `Display`
   instead of its hand-written matches.
