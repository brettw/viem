#!/usr/bin/env bash
# Run native tests against the current Rust archive without touching user settings.
set -euo pipefail
# Wake the display for native UI work, and keep the build/tests awake without
# changing power settings. On AC, -s also prevents maintenance sleep; idle-only
# assertions do not keep an unattended DarkWake session running.
if [ "${VIEM_TESTS_KEEP_AWAKE:-0}" != 1 ]; then
  exec /usr/bin/caffeinate -disu env VIEM_TESTS_KEEP_AWAKE=1 "$0" "$@"
fi
viem_repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$viem_repo_root"
viem_test_config="$(mktemp -d "${TMPDIR:-/tmp}/viem-tests-config.XXXXXX")"
trap 'rm -rf "$viem_test_config"' EXIT
export VIEM_CONFIG_DIR="$viem_test_config"
export CLANG_MODULE_CACHE_PATH="$viem_repo_root/.build/clang-module-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$viem_repo_root/.build/clang-module-cache"
# Match build-mac-app.sh before the ABI check so cc does not rebuild every
# bundled parser when the packaging step changes its deployment target.
export MACOSX_DEPLOYMENT_TARGET=26.0
# Match Rust's test optimization without removing debug assertions or overflow
# checks. The dedicated archive cannot replace the normal debug app's core.
export VIEM_RUST_PROFILE=native-test
cargo run --locked --profile "$VIEM_RUST_PROFILE" --example check_c_abi
"$viem_repo_root/scripts/build-mac-app.sh"
swift test --disable-sandbox "$@"
