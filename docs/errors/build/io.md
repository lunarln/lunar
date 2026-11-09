# build::io: a source file could not be read

The compiler could not read a file it needs. This is reported when:

* the file given on the command line (`lunar check foo.ln`, `lunar build
  foo.ln`, ...) does not exist or cannot be read;
* the standard library crates `core` and `std` cannot be found. By default
  they are embedded in the compiler, so this only happens when the
  `LUNAR_STD_DIR` environment variable points at a directory that does not
  contain `core/lib.ln` and `std/lib.ln`.

Check the path and its spelling, that the file is readable, and, if you set
`LUNAR_STD_DIR`, that it points at the repository's `std` directory (or
unset it to use the embedded copy).

A `mod name;` declaration whose file is missing is a different error:
`err(name::module_file)`.

There is no erroneous example, since the problem is the file system, not
the program's source. For instance, `lunar check missing.ln` reports:

```text
err(build::io): could not read `missing.ln`
```
