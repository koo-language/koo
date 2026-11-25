#!/bin/sh
# Installs the Koo compiler: fetches the build for this system from the project's
# releases, checks it against its published checksum, and puts it in ~/.koo/bin.
#
#     curl -fsSL https://raw.githubusercontent.com/koo-language/koo/main/install.sh | sh
#
# What it reads from the environment:
#
#     KOO_HOME      where to install; ~/.koo if not set
#     KOO_VERSION   the release to install, such as v0.1.0; the latest if not set
#     KOO_FROM      where the archives are, in place of the project's releases
#                   (a URL, which may be a file:// one; this is how the script is tested)
set -eu

say() { printf '%s\n' "$*"; }
fail() { say "koo install: $*" >&2; exit 1; }

case "$(uname -s)" in
    Darwin) system=macos ;;
    Linux) system=linux ;;
    *) fail "there is no build of Koo for $(uname -s) yet; it runs on macOS and Linux" ;;
esac
case "$(uname -m)" in
    arm64 | aarch64) machine=arm64 ;;
    x86_64 | amd64) machine=x86_64 ;;
    *) fail "there is no build of Koo for $(uname -m) yet; it runs on arm64 and x86-64" ;;
esac
if [ "$system-$machine" = macos-x86_64 ]; then
    fail "there is no build of Koo for Intel Macs yet; build it from source, as the README says"
fi

name=koo-$system-$machine
home=${KOO_HOME:-$HOME/.koo}
if [ -n "${KOO_FROM:-}" ]; then
    from=$KOO_FROM
elif [ -n "${KOO_VERSION:-}" ]; then
    from=https://github.com/koo-language/koo/releases/download/$KOO_VERSION
else
    from=https://github.com/koo-language/koo/releases/latest/download
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q "$1" -O "$2"; }
else
    fail "it needs curl or wget to fetch the compiler"
fi
if command -v sha256sum >/dev/null 2>&1; then
    checksum() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    checksum() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    fail "it needs sha256sum or shasum to check what it fetched"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

say "fetching $name from $from"
fetch "$from/$name.tar.gz" "$work/$name.tar.gz" || fail "could not fetch $from/$name.tar.gz"
fetch "$from/$name.tar.gz.sha256" "$work/$name.tar.gz.sha256" || fail "could not fetch the checksum of $name.tar.gz"
wanted=$(cut -d ' ' -f 1 "$work/$name.tar.gz.sha256")
found=$(checksum "$work/$name.tar.gz")
[ "$wanted" = "$found" ] || fail "the archive is not the one that was published: its checksum is $found, and $wanted was expected. Nothing was installed."

tar -xzf "$work/$name.tar.gz" -C "$work"
[ -x "$work/$name/bin/koo" ] || fail "the archive holds no compiler"
mkdir -p "$home/bin"
# Moved into place in one step, so that a compiler that is running is not half replaced.
cp "$work/$name/bin/koo" "$home/bin/koo.new"
mv "$home/bin/koo.new" "$home/bin/koo"

say "installed $("$home/bin/koo" --version) in $home/bin"

# Koo translates a program to LLVM's intermediate language, and clang makes the executable.
if command -v "${KOO_CLANG:-clang}" >/dev/null 2>&1; then
    version=$("${KOO_CLANG:-clang}" --version | head -1)
    major=$(printf '%s' "$version" | sed -n 's/.*version \([0-9][0-9]*\).*/\1/p')
    case "$version" in
        *Apple*) ;;
        *) if [ -n "$major" ] && [ "$major" -lt 15 ]; then
               say "note: Koo needs clang 15 or newer, and yours is $major. Install a newer one; if it has another name, such as clang-19, say so with KOO_CLANG=clang-19."
           fi ;;
    esac
else
    case "$system" in
        macos) say "note: Koo needs clang to build programs. Install it with: xcode-select --install" ;;
        *) say "note: Koo needs clang to build programs. Install it with your system's packages, for instance: sudo apt install clang" ;;
    esac
fi

case ":$PATH:" in
    *":$home/bin:"*) say "run: koo --help" ;;
    *)
        say "add it to your PATH, for instance in your shell's start-up file:"
        say "    export PATH=\"$home/bin:\$PATH\""
        ;;
esac
