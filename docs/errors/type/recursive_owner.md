# type::recursive_owner: a type owns arrays of itself (reserved)

This code is registered but the compiler does not currently emit it.

It is reserved for a type that owns a data stack of values of its own type,
for example a tree node holding a `[Node; ..+]` of children. Each owning
array has its own data stack, so such a type would need an unbounded number
of data stacks.

Today that program is rejected earlier by the rule that array items must be
Dull (`type::array_pointy_items`): a struct containing an owning array is
pointy, so it cannot be the item type of an array, including its own. A
type that contains itself directly is reported as `type::infinite_size`.

Recursive data is built with an array-backed pool: all nodes live in one
array and refer to each other by index.

## Erroneous example

```lunar,ignore
struct Node {
    value: i32,
    children: [Node; ..+],
}
```

## Fixed

The nodes live in one pool and refer to their children by index:

```lunar
struct Node {
    value: i32,
    first_child: Option::<usize>,
    next_sibling: Option::<usize>,
}

fn main() {
    let mut pool: [Node; ..+] = [];
    pool.push(Node { value: 1, first_child: Some(1), next_sibling: None });
    pool.push(Node { value: 2, first_child: None, next_sibling: None });
    println("{pool[0].value} {pool.len()}");
}
```
