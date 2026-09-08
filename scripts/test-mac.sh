#!/usr/bin/env bash
# Run native tests against the current Rust archive without touching user settings.
set -euo pipefail
viem_repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$viem_repo_root"
viem_test_config="$(mktemp -d "${TMPDIR:-/tmp}/viem-tests-config.XXXXXX")"
trap 'rm -rf "$viem_test_config"' EXIT
export VIEM_CONFIG_DIR="$viem_test_config"
export CLANG_MODULE_CACHE_PATH="$viem_repo_root/.build/clang-module-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$viem_repo_root/.build/clang-module-cache"
"$viem_repo_root/scripts/build-mac-app.sh"
swift test --disable-sandbox "$@"
