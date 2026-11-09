# build::missing_lang_item: the standard library lacks an item the compiler needs

Some language features are implemented by items of the standard library
that the compiler finds by a `#[lang = "..."]` attribute: the operator
traits in `core::ops` and `core::cmp` (`add`, `eq`, `ord`, ...), `range` for
range values, `iterator` and `option` for `for` loops over iterators, and
`option` for the `?` operator. This error means the feature was used, but
the loaded `core` does not define the item.

The standard library embedded in the compiler always defines them, so this
only happens when `LUNAR_STD_DIR` points at a modified or incomplete copy of
the standard library. Unset `LUNAR_STD_DIR`, or point it at the repository's
`std` directory, or restore the missing `#[lang = "..."]` attribute.

There is no erroneous example, since a program cannot cause this error on
its own. With a `core` whose `Range` struct lost its `#[lang = "range"]`
attribute, `let r = 0..3;` reports:

```text
err(build::missing_lang_item): a range value needs the lang item `range`, which the standard library does not define
```
