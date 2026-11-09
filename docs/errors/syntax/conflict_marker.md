# syntax::conflict_marker: the file still contains version control conflict markers

A line starts with `<<<<<<<`, `|||||||`, `=======` or `>>>>>>>`. Git (and
other version control tools) write these markers into a file when a merge,
rebase or cherry-pick could not combine two changes to the same lines:

```text
<<<<<<< HEAD
    code you already had
||||||| base            (only with `merge.conflictStyle = diff3`)
    the original code both sides changed
=======
    incoming code
>>>>>>> feature-branch
```

Which side is which depends on the operation. In a **merge**, the top
section is the code you already had checked out and the bottom section is
the branch being merged in. In a **rebase**, it is the other way around: the
top section is the code being rebased onto, and the bottom section is your
own commit being replayed.

A file with conflict markers is not parsed at all. Instead of a cascade of
syntax errors from two half-programs, you get this error once per conflict
region, with each marker labeled.

To fix it, decide what the code should be (one side, the other, or a
combination), then delete every marker line.

## Erroneous example

```lunar
fn greet(): str {
<<<<<<< HEAD
    return "hello";
=======
    return "hi there";
>>>>>>> feat/friendlier
}

fn main() {
    println("{greet()}");
}
```

## Fixed

The incoming change was kept and the marker lines removed:

```lunar
fn greet(): str {
    return "hi there";
}

fn main() {
    println("{greet()}");
}
```
