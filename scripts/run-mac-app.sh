#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")

configuration=${1:-debug}
"$script_dir/build-mac-app.sh" "$configuration"

# A new invocation would otherwise forward to the old process, leaving the
# freshly built executable unused. Check after building so errors never hide
# compiler failures and the user can quit Viem while the build is running.
if running_pids=$(pgrep -u "$(id -u)" -x Viem); then
    printf 'Viem is already running (PID: %s). Quit it and rerun this command.\n' "$running_pids" >&2
    exit 1
else
    status=$?
    if [ "$status" -ne 1 ]; then
        echo "Could not check for an existing Viem instance; refusing to launch." >&2
        exit "$status"
    fi
fi

# The build runs in its own process, so this still inherits the caller's cwd.
if [ "$configuration" = release ]; then
    "$project_dir/.build/release-app/Viem.app/Contents/MacOS/Viem" "$project_dir/docs/markdown_demo.md" &
else
    "$project_dir/.build/Viem.app/Contents/MacOS/Viem" "$project_dir/docs/markdown_demo.md" &
fi
