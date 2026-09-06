#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")
configuration=${1:-debug}

case "$configuration" in
    debug)
        cargo_args=""
        ;;
    release)
        cargo_args="--release"
        ;;
    *)
        echo "usage: $0 [debug|release]" >&2
        exit 64
        ;;
esac

cd "$project_dir"
module_cache_dir="$project_dir/.build/clang-module-cache"
mkdir -p "$module_cache_dir"
export CLANG_MODULE_CACHE_PATH="$module_cache_dir"
export SWIFTPM_MODULECACHE_OVERRIDE="$module_cache_dir"

if [ -n "$cargo_args" ]; then
    cargo build "$cargo_args"
else
    cargo build
fi

# SwiftPM sees the Rust archive only as an unsafe linker argument, so archive
# mtime changes are not part of its dependency graph. Recompile the tiny C shim
# to force a relink against the just-built core on every bundled-app build.
touch "$project_dir/src/mac/CEvimCore/shim.c"

EVIM_RUST_PROFILE="$configuration" swift build --disable-sandbox -c "$configuration"
swift_bin_dir=$(EVIM_RUST_PROFILE="$configuration" swift build --disable-sandbox -c "$configuration" --show-bin-path)

app_bundle="$project_dir/.build/eVim.app"
contents_dir="$app_bundle/Contents"
macos_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"

mkdir -p "$macos_dir" "$resources_dir"
cp "$swift_bin_dir/eVim" "$macos_dir/eVim"
cp "$project_dir/src/mac/App/Resources/Info.plist" "$contents_dir/Info.plist"

codesign --force --sign - "$app_bundle"
echo "$app_bundle"
