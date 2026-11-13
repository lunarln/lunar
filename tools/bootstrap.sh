#!/usr/bin/env bash
# Bootstrap the compiler in craters/ from a stage 0 compiler:
#
#   stage 1 = craters/ built by STAGE0, with STAGE0's own standard library
#   stage 2 = craters/ built by stage 1, with the in-tree std/
#   stage 3 = craters/ built by stage 2, with the in-tree std/
#
# Stages 1 and 2 are the same program built by different compilers, so
# stages 2 and 3 must be identical: the same LLVM IR and the same bytes.
# Stage 2 is the compiler that gets released.
#
# Stage 1 is built with the standard library STAGE0 was released with (its
# sysroot), not the in-tree one: std/ may use language features that only
# the in-tree compiler has, and craters/ may use only what STAGE0 supports.
#
# Usage: tools/bootstrap.sh STAGE0 OUT_DIR
# Linking the compiler needs LLVM's shared library: set LUNAR_LIB_PATH to
# its directory (see README.md).
set -euo pipefail

if [ $# -ne 2 ]; then
    echo "usage: $0 STAGE0 OUT_DIR" >&2
    exit 2
fi
stage0=$1
mkdir -p "$2"
# Native (Windows-style on Windows) absolute paths: the compilers are not
# MSYS programs.
native() { (cd "$1" && { pwd -W 2>/dev/null || pwd; }); }
out=$(native "$2")
root=$(native "$(dirname "$0")/..")
exe=lunar
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) exe=lunar.exe ;;
esac

# Each stage gets the same file name in its own directory: on macOS the
# linker's ad hoc code signature records it.
mkdir -p "$out/stage1" "$out/stage2" "$out/stage3"
cd "$root/craters"

echo "== stage 1: $stage0 ($("$stage0" --version)), its own std"
env -u LUNAR_STD_DIR "$stage0" build --release -p lunar -o "$out/stage1/$exe"

export LUNAR_STD_DIR="$root/std"
echo "== stage 2: stage 1, in-tree std"
"$out/stage1/$exe" build --release -p lunar -o "$out/stage2/$exe" --emit-llvm "$out/stage2.ll"
echo "== stage 3: stage 2, in-tree std"
"$out/stage2/$exe" build --release -p lunar -o "$out/stage3/$exe" --emit-llvm "$out/stage3.ll"

if ! cmp -s "$out/stage2.ll" "$out/stage3.ll"; then
    echo "error: stages 2 and 3 have different LLVM IR:" >&2
    diff "$out/stage2.ll" "$out/stage3.ll" | head -20 >&2 || true
    exit 1
fi
if ! cmp "$out/stage2/$exe" "$out/stage3/$exe"; then
    echo "error: stages 2 and 3 have the same IR but link to different bytes" >&2
    exit 1
fi
echo "== stages 2 and 3 are identical: $("$out/stage2/$exe" --version)"
