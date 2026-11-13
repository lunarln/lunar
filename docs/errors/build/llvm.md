# build::llvm: code generation with LLVM failed

The program passed all checks, but generating machine code with LLVM
failed. This happens when the host target is not supported by the LLVM
library the compiler is using, when the object or IR file cannot be written,
or when the compiler produced invalid LLVM IR or hit a construct code
generation does not support. The last cases are reported as `internal
compiler error` and are bugs in the compiler, not in the program.

What to do:

* for "could not write object file" or "could not write LLVM IR", check that
  the output directory exists and is writable;
* for "unsupported target", make sure the compiler is linked against an
  LLVM 23 build that includes the host target;
* for an internal compiler error, please report it with the program that
  triggers it. Reducing the program to the smallest one that still fails
  helps.

There is no erroneous example: valid programs should never cause this.
`lunar check` does not run code generation and never reports it. Further
detail, such as the LLVM verifier's output for an internal compiler error,
is shown in a note under the error.
