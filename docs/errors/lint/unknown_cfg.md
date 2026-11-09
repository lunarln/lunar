# lint::unknown_cfg: a `cfg` condition uses a name or value that is never set

`#[cfg(...)]` keeps or removes an item depending on the target and build.
The compiler sets exactly these options:

* `unix` and `windows`: the target family;
* `test`: set by `lunar test`;
* `target_os = "..."`: `"linux"`, `"macos"`, `"windows"`, and the other
  operating systems Rust names the same way (`"freebsd"`, `"ios"`, ...);
* `target_family = "..."`: `"unix"` or `"windows"`;
* `target_arch = "..."`: such as `"x86_64"` or `"aarch64"`.

They are combined with `not(...)`, `all(...)` and `any(...)`.

Any other name, such as `linux` on its own, is never set, so the condition
is always false and the item is silently compiled out on every target. This
warning reports such names, unknown `target_os` and `target_family` values,
and options written in the wrong form (`target_os` without a value, or
`test = "..."` with one). It is a warning, not an error.

## Erroneous example

```lunar
#[cfg(linux)]
fn setup() {
    println("setting up for Linux");
}

fn main() {}
```

## Fixed

The operating system is tested with `target_os`:

```lunar
#[cfg(target_os = "linux")]
fn setup() {
    println("setting up for Linux");
}

fn main() {}
```
