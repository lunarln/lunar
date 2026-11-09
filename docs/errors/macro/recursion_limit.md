# macro::recursion_limit: macro expansions nest too deeply

A macro may expand to invocations of macros, including itself, but
expansions may nest at most 128 deep. Going past that almost always means
a recursive macro has no rule that stops the recursion: every expansion
invokes the macro again with input that matches the same rule.

A recursive macro needs a base rule, tried first, for the input where the
recursion ends, and the recursive rule must make its input smaller each
time.

## Erroneous example

```lunar
macro count($x:expr) { count!($x) }

fn main() {
    println("{count!(1)}");
}
```

## Fixed

```lunar
macro count {
    () => { 0 },
    ($x:expr $(, $rest:expr)*) => { 1 + count!($($rest),*) },
}

fn main() {
    println("{count!(7, 8, 9)}");
}
```
