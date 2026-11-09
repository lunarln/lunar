# type::bad_test: a `#[test]` function has the wrong signature

`lunar test` calls each `#[test]` function with no arguments and treats the
test as passed if it returns normally. A test function must therefore take
no parameters, have no generic parameters, and return `()`.

To report failure, call `assert(cond, msg)`, `assert_eq(a, b, msg)` or
`panic` inside the test instead of returning a value.

## Erroneous example

```lunar
#[test]
fn adds(): bool {
    return 1 + 1 == 2;
}

fn main() {}
```

## Fixed

The test now returns `()` and checks its condition with `assert`:

```lunar
#[test]
fn adds() {
    assert(1 + 1 == 2, "1 + 1 should be 2");
}

fn main() {}
```
