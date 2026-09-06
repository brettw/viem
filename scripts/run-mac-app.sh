#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(dirname -- "$script_dir")

"$script_dir/build-mac-app.sh" "${1:-debug}"
open "$project_dir/.build/eVim.app"
