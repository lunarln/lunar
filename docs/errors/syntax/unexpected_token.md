# syntax::unexpected_token: the parser found a token it did not expect here

The parser reached a token that cannot appear at this point in the program.
The message lists what it expected instead, for example ``expected one of
`:`, `;`, `{`, found `->` ``. When the file ends too early, the token found
is "end of file".

Common causes are syntax borrowed from another language (Lunar writes a
return type as `fn f(): T`, not `fn f() -> T`, and generic arguments always as
`::<T>`), a missing closing bracket, or a stray token left over from an edit.

The same code is used for a few structural errors found after parsing:

* an item other than a function inside a `trait` or `impl` block;
* anything other than a body-less function declaration inside an
  `extern "C" { }` block;
* generic arguments on a field access, such as `p.x::<i32>`.

Only the first problem in a region is usually meaningful; later errors can be
consequences of the parser recovering from the first one.

## Erroneous example

```lunar
fn add(a: i32, b: i32) -> i32 {
    return a + b;
}

fn main() {
    println("{add(1, 2)}");
}
```

## Fixed

The return type follows a `:`:

```lunar
fn add(a: i32, b: i32): i32 {
    return a + b;
}

fn main() {
    println("{add(1, 2)}");
}
```
