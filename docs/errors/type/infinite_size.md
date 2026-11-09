# type::infinite_size: a type contains itself and would have infinite size

A struct or enum contains a field of its own type, directly or through other
structs, enums or tuples. Such a value would have to contain a complete copy
of itself, so it has no finite size.

Lunar has no heap and no boxes, so the cycle can only be broken by a level of
indirection that stores the value elsewhere: a raw pointer (`*T`), or an
owning array or slice. Since array items must be Dull, the usual way to build
recursive data such as lists and trees is an array-backed pool: keep all
nodes in one `[Node; ..+]` and refer to other nodes by index.

The same code is reported for a type alias that refers to itself, such as
`type Foo = [Foo; 2];`.

## Erroneous example

```lunar
struct Node {
    value: i32,
    next: Option::<Node>,
}

fn main() {}
```

## Fixed

The node now refers to the next node by its index in a pool:

```lunar
struct Node {
    value: i32,
    next: Option::<usize>,
}

fn main() {
    let mut pool: [Node; ..+] = [];
    pool.push(Node { value: 1, next: None });
    pool.push(Node { value: 2, next: Some(0) });
    println("{pool.len()}");
}
```
