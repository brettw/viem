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
exec python3 "$viem_repo_root/scripts/native-tests.py" "$@"
