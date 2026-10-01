#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")

"$script_dir/build-mac-app.sh" "${1:-debug}"
# The build runs in its own process, so this still inherits the caller's cwd.
"$project_dir/.build/Viem.app/Contents/MacOS/Viem" &
