#!/bin/sh
# Build the libFuzzer targets on the stable toolchain with SanitizerCoverage,
# debug assertions, overflow checks and AddressSanitizer (Rust and the native
# Tree-sitter C code). RUSTC_BOOTSTRAP only enables -Zsanitizer for this build.
# Usage: fuzz/build.sh [--no-asan|--plain] [cargo build args...]
#   --no-asan  coverage-guided, without AddressSanitizer
#   --plain    no coverage or sanitizers, to time or replay inputs; use a
#              separate CARGO_TARGET_DIR, e.g. CARGO_TARGET_DIR=fuzz/target-plain
set -eu
cd "$(dirname "$0")"
mode=asan
case "${1:-}" in
  --no-asan) mode=coverage; shift ;;
  --plain) mode=plain; shift ;;
esac
flags="--cfg fuzzing -Cdebug-assertions -Coverflow-checks -Cforce-frame-pointers=yes"
cflags="-g -fno-omit-frame-pointer"
if [ "$mode" != plain ]; then
  flags="$flags -Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4 \
-Cllvm-args=-sanitizer-coverage-inline-8bit-counters -Cllvm-args=-sanitizer-coverage-pc-table \
-Cllvm-args=-sanitizer-coverage-trace-compares"
  cflags="$cflags -fsanitize-coverage=inline-8bit-counters,pc-table,trace-cmp"
fi
case $mode in
  asan)
    flags="$flags -Zsanitizer=address"
    cflags="$cflags -fsanitize=address"
    export RUSTC_BOOTSTRAP=1
    # Apple clang emits a toolchain-specific ASan ABI check that Rust's LLVM
    # runtime does not provide; upstream clang matches it. Override with CC.
    if [ -z "${CC:-}" ] && [ -x /opt/homebrew/opt/llvm/bin/clang ]; then
      export CC=/opt/homebrew/opt/llvm/bin/clang
    fi ;;
  coverage)
    # The core's cdylib is linked without libFuzzer's coverage callbacks.
    flags="$flags -Clink-arg=-Wl,-undefined,dynamic_lookup" ;;
esac
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
RUSTFLAGS="$flags" CFLAGS="$cflags" \
  cargo build --release --target aarch64-apple-darwin "$@"
