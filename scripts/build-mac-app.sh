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
python3 "$script_dir/vim-runtime.py" verify "$project_dir/assets/vim"
module_cache_dir="$project_dir/.build/clang-module-cache"
mkdir -p "$module_cache_dir"
export CLANG_MODULE_CACHE_PATH="$module_cache_dir"
export SWIFTPM_MODULECACHE_OVERRIDE="$module_cache_dir"
# Keep C dependencies (including bundled syntax parsers) at the same minimum
# macOS version as the Swift package, regardless of the build host's version.
export MACOSX_DEPLOYMENT_TARGET=26.0

if [ -n "$cargo_args" ]; then
    cargo build "$cargo_args"
else
    cargo build
fi

# SwiftPM sees the Rust archive only as an unsafe linker argument, so archive
# mtime changes are not part of its dependency graph. Recompile the tiny C shim
# to force a relink against the just-built core on every bundled-app build.
touch "$project_dir/src/mac/CViemCore/shim.c"

VIEM_RUST_PROFILE="$configuration" swift build --disable-sandbox -c "$configuration"
swift_bin_dir=$(VIEM_RUST_PROFILE="$configuration" swift build --disable-sandbox -c "$configuration" --show-bin-path)

app_bundle="$project_dir/.build/Viem.app"
contents_dir="$app_bundle/Contents"
macos_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"

mkdir -p "$macos_dir" "$resources_dir"
# A previous development build may still be running. Replace its executable
# inode atomically instead of truncating bytes mapped by that process.
cp "$swift_bin_dir/Viem" "$macos_dir/Viem.new"
mv -f "$macos_dir/Viem.new" "$macos_dir/Viem"
cp "$project_dir/src/mac/App/Resources/Info.plist" "$contents_dir/Info.plist"
cp "$project_dir/assets/icon/Viem.icns" "$resources_dir/Viem.icns"
# Theme presets are versioned resources. Profiles copy these when first made;
# the core also carries an independent Midnight fallback for missing resources.
mkdir -p "$resources_dir/themes"
cp "$project_dir/assets/themes/"*.json "$resources_dir/themes/"
# Replace this owned subtree so removed upstream files cannot survive a rebuild.
# All files, including the original license and provenance manifest, are sealed
# into the application signature. No installed Vim is needed at build/run time.
rm -rf "$resources_dir/vim"
cp -R "$project_dir/assets/vim" "$resources_dir/vim"
python3 "$script_dir/vim-runtime.py" verify "$resources_dir/vim"

# The syntax queries are embedded in the Rust library; distribute their
# upstream licenses and attribution inventory alongside the executable.
query_source_dir="$project_dir/src/core/document/syntax/treesitter"
query_license_dir="$resources_dir/Licenses/nvim-treesitter"
mkdir -p "$query_license_dir"
cp "$query_source_dir/"*.LICENSE "$query_source_dir/nvim.NOTICES.md" "$query_license_dir/"

codesign --force --sign - "$app_bundle"
# Nested resource updates do not change the bundle directory's modification
# time. Refresh it so Launch Services notices icon and metadata changes.
touch "$app_bundle"
echo "$app_bundle"
