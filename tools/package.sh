#!/usr/bin/env bash
# Package a built compiler as a release archive, in the install layout:
#
#   lunar-VERSION-TARGET/
#     bin/lunar[.exe]
#     bin/LLVM-C.dll               (Windows: found next to the executable)
#     lib/libLLVM.so.N, ...        (Linux: found through RUNPATH $ORIGIN/../lib)
#     lib/libLLVM.dylib, ...       (macOS: found through @executable_path/../lib)
#     lib/lunar/std/{core,std}/...
#     lib/lunar/docs/errors/...
#     LICENSE-MIT, LICENSE-APACHE, README.md
#     licenses/                    (LLVM's license and those of the libraries
#                                   bundled with it)
#
# The LLVM shared library the compiler was linked against is copied into
# the archive with the libraries it needs that a base system lacks, and the
# executable is pointed at them, so it runs with nothing else installed.
# Then the packaged compiler is tried out: from the archive's directory,
# with no LUNAR_* variables, it must build and run a program.
#
# Usage: tools/package.sh LUNAR_EXE TARGET OUT_DIR
# Writes OUT_DIR/lunar-VERSION-TARGET.{tar.gz,zip} and a .sha256 next to it.
# LLVM_SHLIB_DIR is the directory of LLVM's shared library (on Windows,
# LLVM-C.dll's).
set -euo pipefail

if [ $# -ne 3 ]; then
    echo "usage: $0 LUNAR_EXE TARGET OUT_DIR" >&2
    exit 2
fi
exe=$1
target=$2
mkdir -p "$3"
out=$(cd "$3" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)
version=$("$exe" --version | awk '{print $2}')
name="lunar-$version-$target"
stage="$out/$name"
llvm_tag=llvmorg-20.1.8

case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    MINGW* | MSYS* | CYGWIN*) os=windows ;;
    *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/lib/lunar/docs" "$stage/licenses"
cp "$exe" "$stage/bin/"
cp -R "$root/std" "$stage/lib/lunar/std"
cp -R "$root/docs/errors" "$stage/lib/lunar/docs/errors"
cp "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$root/README.md" "$stage/"
curl -fsSL --retry 3 "https://raw.githubusercontent.com/llvm/llvm-project/$llvm_tag/llvm/LICENSE.TXT" -o "$stage/licenses/LLVM-LICENSE.txt"

# Whether a library that the LLVM library needs is part of every system
# (and must not be bundled): the C and C++ runtimes.
is_system_lib() {
    case "$os" in
        linux)
            case "$1" in
                linux-vdso.so* | ld-linux* | libc.so* | libm.so* | libdl.so* | libpthread.so* | librt.so* | libstdc++.so* | libgcc_s.so*) return 0 ;;
            esac
            return 1
            ;;
        macos)
            case "$1" in
                /usr/lib/* | /System/*) return 0 ;;
            esac
            return 1
            ;;
    esac
}

bundle_linux() {
    local bin="$stage/bin/lunar"
    local llvm_line llvm
    llvm_line=$(ldd "$bin" | awk '/libLLVM/ { print $1 " => " $3 }')
    llvm=${llvm_line##* }
    [ -n "$llvm" ] || { echo "error: $bin does not link libLLVM" >&2; exit 1; }
    # ldd lists every library loaded, directly or not.
    { echo "$llvm_line"; ldd "$llvm"; } | awk '$2 == "=>" && $3 ~ /^\// { print $1, $3 }' | sort -u |
        while read -r soname path; do
            if is_system_lib "$soname"; then
                continue
            fi
            cp -L "$path" "$stage/lib/$soname"
            chmod u+w "$stage/lib/$soname"
            patchelf --set-rpath '$ORIGIN' "$stage/lib/$soname"
            # The Debian package's copyright file is the library's license.
            local pkg
            pkg=$(dpkg -S "$path" 2>/dev/null || dpkg -S "$(readlink -f "$path")" 2>/dev/null || true)
            pkg=${pkg%%:*}
            if [ -n "$pkg" ] && [ -f "/usr/share/doc/$pkg/copyright" ]; then
                cp "/usr/share/doc/$pkg/copyright" "$stage/licenses/$pkg.copyright"
            fi
        done
    patchelf --set-rpath '$ORIGIN/../lib' "$bin"
    echo "bundled:"; ls -l "$stage/lib"
    echo "needs glibc $(objdump -T "$bin" "$stage"/lib/*.so* | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -n 1)" \
        "and libstdc++ $(objdump -T "$stage"/lib/*.so* | grep -o 'GLIBCXX_[0-9.]*' | sort -uV | tail -n 1)"
    # The packaged executable must find LLVM in the package.
    if ! ldd "$bin" | grep libLLVM | grep -qF "$stage/lib/"; then
        echo "error: the packaged lunar does not load LLVM from the package:" >&2
        ldd "$bin" >&2
        exit 1
    fi
}

bundle_macos() {
    local bin="$stage/bin/lunar"
    # Refer to every non-system library as @rpath/<name>, copy it into lib/
    # and do the same for its own dependencies.
    local queue=("$bin") done_list=" "
    while [ ${#queue[@]} -gt 0 ]; do
        local file=${queue[0]}
        queue=("${queue[@]:1}")
        local id
        id=$(otool -D "$file" | tail -n +2)
        local dep
        for dep in $(otool -L "$file" | tail -n +2 | awk '{ print $1 }'); do
            if [ "$dep" = "$id" ] || is_system_lib "$dep"; then
                continue
            fi
            local base
            base=$(basename "$dep")
            install_name_tool -change "$dep" "@rpath/$base" "$file"
            case "$done_list" in
                *" $base "*) continue ;;
            esac
            done_list="$done_list$base "
            local src=$dep
            case "$dep" in
                @rpath/*) src="$LLVM_SHLIB_DIR/$base" ;;
            esac
            cp -L "$src" "$stage/lib/$base"
            chmod u+w "$stage/lib/$base"
            install_name_tool -id "@rpath/$base" "$stage/lib/$base"
            # Its license: the Homebrew formula's, if it has one.
            local cellar
            cellar=$(cd "$(dirname "$(readlink -f "$src")")/.." && pwd)
            local lic
            for lic in "$cellar"/LICENSE* "$cellar"/COPYING*; do
                if [ -f "$lic" ]; then
                    cp "$lic" "$stage/licenses/${base%%.*}-$(basename "$lic")"
                fi
            done
            queue+=("$stage/lib/$base")
        done
    done
    # Only the package's lib/ on the run path, not the build machine's.
    local rp
    for rp in $(otool -l "$bin" | awk '/cmd LC_RPATH/ { getline; getline; print $2 }'); do
        install_name_tool -delete_rpath "$rp" "$bin"
    done
    install_name_tool -add_rpath "@executable_path/../lib" "$bin"
    # Changing a Mach-O file invalidates its signature; sign them again
    # (ad hoc, as the linker did).
    local f
    for f in "$bin" "$stage"/lib/*.dylib; do
        codesign --force --sign - "$f"
    done
    echo "bundled:"; ls -l "$stage/lib"
    otool -L "$bin"
}

case "$os" in
    linux) bundle_linux ;;
    macos) bundle_macos ;;
    windows) cp "$LLVM_SHLIB_DIR/LLVM-C.dll" "$stage/bin/" ;;
esac

# Try the package out: no LUNAR_* variables, a fresh directory, and a
# program to build, run and test.
try=$(mktemp -d)
cp "$root/examples/arrays.ln" "$try/"
(
    cd "$try"
    unset LUNAR_STD_DIR LUNAR_LIB_PATH LUNAR_HOME LUNAR_CC
    "$stage/bin/lunar" --version
    "$stage/bin/lunar" run arrays.ln > arrays.out
    diff arrays.out "$root/examples/arrays.out"
    "$stage/bin/lunar" test arrays.ln
    "$stage/bin/lunar" explain type::mismatch > /dev/null
)
rm -rf "$try"

cd "$out"
if [ "$os" = windows ]; then
    archive="$name.zip"
    rm -f "$archive"
    7z a -tzip -bd -mx=9 "$archive" "$name" > /dev/null
else
    archive="$name.tar.gz"
    tar -czf "$archive" "$name"
fi
if command -v sha256sum > /dev/null; then
    hash=$(sha256sum < "$archive" | awk '{ print $1 }')
else
    hash=$(shasum -a 256 < "$archive" | awk '{ print $1 }')
fi
# The format of `sha256sum`: `sha256sum -c` checks it.
printf '%s  %s\n' "$hash" "$archive" > "$archive.sha256"
rm -rf "$stage"
echo "packaged $out/$archive"
cat "$archive.sha256"
