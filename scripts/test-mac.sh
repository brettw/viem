#!/usr/bin/env bash
# Run native tests against the current Rust archive without touching user settings.
set -euo pipefail
evim_repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$evim_repo_root"
evim_test_config="$(mktemp -d "${TMPDIR:-/tmp}/evim-tests-config.XXXXXX")"
trap 'rm -rf "$evim_test_config"' EXIT
export EVIM_CONFIG_DIR="$evim_test_config"
export CLANG_MODULE_CACHE_PATH="$evim_repo_root/.build/clang-module-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$evim_repo_root/.build/clang-module-cache"
"$evim_repo_root/scripts/build-mac-app.sh"
swift test --disable-sandbox "$@"
