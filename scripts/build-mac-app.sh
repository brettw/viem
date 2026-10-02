#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")
configuration=${1:-debug}

case "$configuration" in
    debug|release)
        ;;
    *)
        echo "usage: $0 [debug|release]" >&2
        exit 64
        ;;
esac

rust_profile=${VIEM_RUST_PROFILE:-$configuration}

cd "$project_dir"
python3 "$script_dir/vim-runtime.py" verify "$project_dir/assets/vim"
module_cache_dir="$project_dir/.build/clang-module-cache"
mkdir -p "$module_cache_dir"
export CLANG_MODULE_CACHE_PATH="$module_cache_dir"
export SWIFTPM_MODULECACHE_OVERRIDE="$module_cache_dir"
# Keep C dependencies (including bundled syntax parsers) at the same minimum
# macOS version as the Swift package, regardless of the build host's version.
export MACOSX_DEPLOYMENT_TARGET=26.0

if [ "$rust_profile" = debug ]; then
    cargo build
else
    cargo build --profile "$rust_profile"
fi

swift_bin_dir=$(VIEM_RUST_PROFILE="$rust_profile" swift build --disable-sandbox -c "$configuration" --show-bin-path)
rust_archive="$project_dir/target/$rust_profile/libviem_core.a"
swift_binary="$swift_bin_dir/Viem"
link_stamp="$project_dir/.build/rust-link-$configuration.sha256"

# SwiftPM sees the archive only as an unsafe linker argument, so it cannot
# detect changed archive bytes. SHA256 output includes the selected path too:
# switching Rust profiles must relink even if their archives happen to match.
archive_fingerprint=$(shasum -a 256 "$rust_archive")
binary_fingerprint="missing $swift_binary"
if [ -f "$swift_binary" ]; then
    binary_fingerprint=$(shasum -a 256 "$swift_binary")
fi
current_fingerprint=$(printf '%s\n%s\n' "$archive_fingerprint" "$binary_fingerprint")
# Also verify the linked output, since a manual Swift build can replace it
# without updating this script's last-successful-link stamp.
if [ ! -f "$link_stamp" ] || [ "$(cat "$link_stamp")" != "$current_fingerprint" ]; then
    touch "$project_dir/src/mac/CViemCore/shim.c"
fi

VIEM_RUST_PROFILE="$rust_profile" swift build --disable-sandbox -c "$configuration"
# Do not bless a link if another build changed the archive while Swift ran.
if [ "$(shasum -a 256 "$rust_archive")" != "$archive_fingerprint" ]; then
    echo "Rust archive changed during Swift build; rerun the build." >&2
    exit 1
fi
binary_fingerprint=$(shasum -a 256 "$swift_binary")
link_stamp_candidate=$(mktemp "$link_stamp.XXXXXX")
trap 'rm -f "$link_stamp_candidate"' 0
printf '%s\n%s\n' "$archive_fingerprint" "$binary_fingerprint" > "$link_stamp_candidate"
mv -f "$link_stamp_candidate" "$link_stamp"
trap - 0

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
# App-local fonts and their original license/attribution files. Replace the
# owned subtree so removed faces cannot survive a rebuild.
rm -rf "$resources_dir/fonts"
cp -R "$project_dir/assets/fonts" "$resources_dir/fonts"
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
