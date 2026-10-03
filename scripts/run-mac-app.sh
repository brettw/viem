#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")

configuration=${1:-debug}
"$script_dir/build-mac-app.sh" "$configuration"
# The build runs in its own process, so this still inherits the caller's cwd.
if [ "$configuration" = release ]; then
    "$project_dir/.build/release-app/Viem.app/Contents/MacOS/Viem" &
else
    "$project_dir/.build/Viem.app/Contents/MacOS/Viem" &
fi
