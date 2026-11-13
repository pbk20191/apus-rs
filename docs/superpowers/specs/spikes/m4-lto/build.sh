#!/bin/bash
# Cross-language LTO between Rust and Swift: both directions inline.
set -e
cd "$(dirname "$0")"; mkdir -p out; cd out
T=$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin; SDK=$(xcrun --show-sdk-path)
# Swift → LLVM IR; drop what upstream LLVM cannot codegen or what blocks inlining; back to (thin) bitcode with Apple clang.
swiftc -O -parse-as-library -module-name Sw -import-objc-header ../rs.h -emit-ir ../sw.swift -o sw.raw.ll
sed -E -e 's/ "probe-stack"="__chkstk_darwin"//' -e 's/ "target-features"="[^"]*"//' \
       -e 's/^attributes (#[0-9]+) = \{ noinline \}/attributes \1 = { nounwind }/' sw.raw.ll > sw.ll
clang -flto=thin -x ir sw.ll -c -o sw.bc 2>/dev/null
# Rust → static library whose own objects are bitcode.
rustc -C opt-level=3 -C linker-plugin-lto -C panic=abort -C target-cpu=apple-m1 --crate-type staticlib ../lib.rs -o librs.a
# Stage 1: the LLVM that rustc ships (rust-lld) runs the LTO and keeps the objects.
rm -rf lto; mkdir lto
"$T/gcc-ld/ld64.lld" -arch arm64 -platform_version macos 14.0 27.0 -dylib -undefined dynamic_lookup --lto-O3 \
  -object_path_lto "$PWD/lto" -all_load librs.a sw.bc -o stage1.dylib 2>/dev/null
# Stage 2: Apple's linker makes the real binary against the SDK.
clang lto/*.o librs.a -L/usr/lib/swift -L"$SDK/usr/lib/swift" -liconv -o demo
./demo
for f in _rust_calls_swift _rust_swift_rust _swift_mul_add; do echo "$f:"; otool -tvV -p "$f" demo | sed -n '3,8p'; done
