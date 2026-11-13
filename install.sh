#!/bin/sh
# Install the Lunar compiler from a GitHub release, on Linux (x86_64) or
# macOS (Apple silicon):
#
#   curl -fsSL https://raw.githubusercontent.com/lunarln/lunar/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/lunarln/lunar/main/install.sh | sh -s -- --version 0.1.0
#   sh install.sh [--prefix DIR] [--version X.Y.Z]
#
# It downloads the release archive for this platform, checks it against its
# SHA-256 checksum and unpacks it into the prefix (default ~/.lunar, or
# $LUNAR_PREFIX): the compiler in bin/, its standard library and LLVM in
# lib/. The version defaults to the latest release (or $LUNAR_VERSION).
set -eu

repo=lunarln/lunar
prefix=${LUNAR_PREFIX:-$HOME/.lunar}
version=${LUNAR_VERSION:-}

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) [ $# -ge 2 ] || die "--prefix needs a directory"; prefix=$2; shift 2 ;;
        --prefix=*) prefix=${1#*=}; shift ;;
        --version) [ $# -ge 2 ] || die "--version needs a version"; version=$2; shift 2 ;;
        --version=*) version=${1#*=}; shift ;;
        -h | --help) say "usage: install.sh [--prefix DIR (default ~/.lunar)] [--version X.Y.Z (default: the latest)]"; exit 0 ;;
        *) die "unknown argument '$1' (try --help)" ;;
    esac
done
version=${version#v}

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
    Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-gnu ;;
    Darwin/arm64 | Darwin/aarch64) target=aarch64-apple-darwin ;;
    *) die "there is no prebuilt Lunar for $os on $arch; build it from source (see https://github.com/$repo#building)" ;;
esac

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
    fetch_text() { curl -fsSL --retry 3 "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
    fetch_text() { wget -q -O - "$1"; }
else
    die "curl or wget is needed to download Lunar"
fi

if [ -z "$version" ]; then
    version=$(fetch_text "https://api.github.com/repos/$repo/releases/latest" |
        sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1) ||
        true
    [ -n "$version" ] || die "could not find the latest release of $repo; pass --version"
fi

archive="lunar-$version-$target.tar.gz"
url="https://github.com/$repo/releases/download/v$version/$archive"
tmp=$(mktemp -d 2>/dev/null || mktemp -d -t lunar-install)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading Lunar $version for $target"
fetch "$url" "$tmp/$archive" || die "could not download $url"
fetch "$url.sha256" "$tmp/$archive.sha256" || die "could not download $url.sha256"

expected=$(awk '{ print $1; exit }' "$tmp/$archive.sha256")
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$archive" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$tmp/$archive" | awk '{ print $1 }')
else
    die "sha256sum or shasum is needed to check the download"
fi
[ "$expected" = "$actual" ] || die "checksum mismatch for $archive: got $actual, expected $expected"

tar -xzf "$tmp/$archive" -C "$tmp"
src="$tmp/lunar-$version-$target"
[ -x "$src/bin/lunar" ] || die "$archive does not have bin/lunar"

# Replace the standard library (files may have been removed); the rest is
# overwritten in place.
mkdir -p "$prefix/bin" "$prefix/lib" "$prefix/share/doc/lunar"
rm -rf "$prefix/lib/lunar"
cp -R "$src/bin/." "$prefix/bin/"
cp -R "$src/lib/." "$prefix/lib/"
cp -R "$src/LICENSE-MIT" "$src/LICENSE-APACHE" "$src/README.md" "$src/licenses" "$prefix/share/doc/lunar/"

say "installed $("$prefix/bin/lunar" --version) to $prefix"

case ":${PATH:-}:" in
    *":$prefix/bin:"*) ;;
    *)
        say ""
        say "Add $prefix/bin to your PATH, for example in ~/.profile, ~/.bashrc or ~/.zshrc:"
        say ""
        say "    export PATH=\"$prefix/bin:\$PATH\""
        ;;
esac

# Programs are linked by the system's C compiler driver.
if ! command -v cc >/dev/null 2>&1 && [ -z "${LUNAR_CC:-}" ]; then
    say ""
    if [ "$os" = Darwin ]; then
        say "note: lunar links programs with \`cc\`, which is missing: install the Xcode command line tools (xcode-select --install)"
    else
        say "note: lunar links programs with \`cc\`, which is missing: install a C toolchain (e.g. build-essential, gcc or clang)"
    fi
fi
