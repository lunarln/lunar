# name::inconsistent_binding: a variable is bound differently in the alternatives of a `|` pattern

The alternatives of a `|` pattern all bind the same variable, so they must
agree on how they bind it: a variable is `mut` in every alternative or in
none. (Its type must be the same in every alternative too; a different type
is reported as `err(type::mismatch)`.)

The error points at the binding that differs and at the first binding of
the variable.

## Erroneous example

```lunar
enum Pair {
    Left(i32, i32),
    Right(i32, i32),
}

fn bump(p: Pair): i32 {
    return match p {
        Pair::Left(mut x, _) | Pair::Right(_, x) => {
            x += 1;
            x
        }
    };
}

fn main() {
    println("{bump(Pair::Right(1, 2))}");
}
```

## Fixed

Both alternatives bind `x` with `mut`:

```lunar
enum Pair {
    Left(i32, i32),
    Right(i32, i32),
}

fn bump(p: Pair): i32 {
    return match p {
        Pair::Left(mut x, _) | Pair::Right(_, mut x) => {
            x += 1;
            x
        }
    };
}

fn main() {
    println("{bump(Pair::Right(1, 2))}");
}
```
