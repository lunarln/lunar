# build::link: linking the executable failed

After a program has been compiled to an object file, `lunar build`, `lunar
run` and `lunar test` link it into an executable with the platform's
toolchain. This error means that step failed: no linker was found, the
linker could not be started, or it ran and reported an error.

The linker used is:

* Windows: `clang` if it is on `PATH`, otherwise MSVC `link.exe` if it is
  on `PATH` (as in a Developer PowerShell for Visual Studio);
* Linux and macOS: the system C compiler driver `cc`;
* everywhere: the program named by the `LUNAR_CC` environment variable, if
  set. It is invoked like a C compiler: `$LUNAR_CC file.o -o out`.

To fix it, install a linker or point `LUNAR_CC` at one:

* Windows: the Visual Studio Build Tools (the C++ workload, for the C
  runtime and the Windows SDK) and LLVM `clang`, which finds them by
  itself; or the Build Tools alone, running `lunar` from a Developer
  PowerShell so that `link.exe` is on `PATH`;
* macOS: the Xcode command line tools (`xcode-select --install`);
* Linux: gcc or clang (on Debian and Ubuntu, `build-essential`).

If the linker
runs but fails, its output is shown in a note under the error; undefined
symbols usually come from an `extern` function that does not exist in the C
runtime or the system libraries that are linked by default.

The program's source is not at fault, so there is no erroneous example.
`lunar check` never links and never reports this error. With
`LUNAR_CC=missing-cc`, `lunar build` reports:

```text
err(build::link): could not run the linker `missing-cc`: program not found
  help: install a linker, or point `LUNAR_CC` at one
```
