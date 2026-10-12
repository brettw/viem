#!/bin/sh
# Run one libFuzzer campaign in fork mode, collecting distinct failures in
# artifacts/ and growing corpus/<target>. Usage:
#   fuzz/run.sh TARGET [WORKERS] [SECONDS] [extra libFuzzer flags...]
set -eu
cd "$(dirname "$0")"
target=$1; workers=${2:-2}; seconds=${3:-3600}
shift; [ $# -gt 0 ] && shift; [ $# -gt 0 ] && shift
case $target in
  open_document) max_len=65536; timeout=25 ;;
  markdown_edit) max_len=16384; timeout=25 ;;
  clipboard_html) max_len=32768; timeout=25 ;;
  treesitter) max_len=16384; timeout=25 ;;
  syntax_highlight) max_len=8192; timeout=60 ;;
  vim_highlight) max_len=8192; timeout=60; rss=8192 ;;
  *) max_len=4096; timeout=25 ;;
esac
rss=${rss:-4096}
bin=${FUZZ_BIN:-target/aarch64-apple-darwin/release}/$target
mkdir -p "corpus/$target" artifacts
export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=0:abort_on_error=1:symbolize=1:allocator_may_return_null=0}"
exec "$bin" "corpus/$target" -fork="$workers" -ignore_crashes=1 -ignore_timeouts=1 \
  -ignore_ooms=1 -max_total_time="$seconds" -max_len="$max_len" -timeout="$timeout" \
  -rss_limit_mb="$rss" -malloc_limit_mb=2048 -artifact_prefix="artifacts/$target-" \
  -print_final_stats=1 "$@"
