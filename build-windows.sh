#!/bin/bash
#
# Cross-compile halfremembered-launcher for Windows from Linux/WSL
#
# PREREQUISITES (install before running this script):
#
# 1. Rust Windows target:
#    rustup target add x86_64-pc-windows-gnu
#
# 2. MinGW-w64 cross-compiler toolchain & NASM:
#    - Arch Linux:   pacman -S mingw-w64-gcc nasm
#    - Ubuntu/Debian: apt install gcc-mingw-w64-x86-64 nasm
#    - Fedora:       dnf install mingw64-gcc nasm
#
# OUTPUT:
#    Fully static exe at: target/x86_64-pc-windows-gnu/release/halfremembered-launcher.exe

echo "Building halfremembered-launcher for Windows with static linking..."

set -e

target="x86_64-pc-windows-gnu"

# Static linking flags - no DLLs needed
export RUSTFLAGS="-C target-feature=+crt-static -C link-arg=-static"

cargo build --release --target $target --bin halfremembered-launcher

echo
echo "$(pwd)/target/$target/release/halfremembered-launcher.exe"
echo
