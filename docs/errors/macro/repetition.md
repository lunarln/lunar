# macro::repetition: a repetition in a macro body cannot be expanded

A `$( ... )` repetition in a macro's body repeats once for each match of
the variables it uses that were matched inside a repetition. This error is
reported when:

* two such variables used in the same repetition matched a different number
  of times, so it is not clear how often to repeat;
* a variable matched inside a repetition is used outside one (it has many
  values, not one);
* a repetition uses no variable that was matched in a repetition.

## Erroneous example

```lunar
macro dot($($a:expr),*; $($b:expr),*) {
    0 $(+ $a * $b)*
}

fn main() {
    println("{dot!(1, 2; 3)}");
}
```

## Fixed

Both lists have the same length:

```lunar
macro dot($($a:expr),*; $($b:expr),*) {
    0 $(+ $a * $b)*
}

fn main() {
    println("{dot!(1, 2; 3, 4)}");
}
```
