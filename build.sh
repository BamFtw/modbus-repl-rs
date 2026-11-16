#!/usr/bin/env bash
# Build modbus-repl in release mode.
#
# Usage:
#   ./build.sh            build for the current platform
#   ./build.sh linux      static Linux binary   (x86_64-unknown-linux-musl)
#   ./build.sh windows    Windows executable    (x86_64-pc-windows-gnu)
#   ./build.sh all        both of the above
#
# Resulting executables are copied to ./dist/

set -euo pipefail

cd "$(dirname "$0")"

LINUX_TARGET=x86_64-unknown-linux-musl
WINDOWS_TARGET=x86_64-pc-windows-gnu

mkdir -p dist

build_target() {
    local target="$1" bin="$2" out="$3"

    if command -v rustup >/dev/null 2>&1; then
        rustup target add "$target" >/dev/null
    fi

    # When cross-compiling to Windows from a non-Windows host, use mingw-w64.
    if [[ "$target" == "$WINDOWS_TARGET" && "$(uname -s)" != MINGW* && "$(uname -s)" != MSYS* ]]; then
        export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
    fi

    echo ">> building $target"
    cargo build --release --target "$target"
    cp "target/$target/release/$bin" "dist/$out"
    echo ">> dist/$out"
}

build_native() {
    echo ">> building for host platform"
    cargo build --release
    local bin=modbus-repl
    [[ -f target/release/modbus-repl.exe ]] && bin=modbus-repl.exe
    cp "target/release/$bin" "dist/$bin"
    echo ">> dist/$bin"
}

case "${1:-native}" in
    native)  build_native ;;
    linux)   build_target "$LINUX_TARGET" modbus-repl modbus-repl-linux-x86_64 ;;
    windows) build_target "$WINDOWS_TARGET" modbus-repl.exe modbus-repl-windows-x86_64.exe ;;
    all)
        build_target "$LINUX_TARGET" modbus-repl modbus-repl-linux-x86_64
        build_target "$WINDOWS_TARGET" modbus-repl.exe modbus-repl-windows-x86_64.exe
        ;;
    -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' ;;
    *) echo "unknown option '$1' (use: native, linux, windows, all)" >&2; exit 2 ;;
esac
