#!/bin/sh
# Record Vim's output for Vim script fixtures.
#
# Usage: scripts/vimscript-oracle.sh [fixture.vim ...]
#
# Runs each fixture in Vim 9 (VIEM_VIM_ORACLE, or vim on PATH) without user
# configuration, in a UTF-8 locale, and writes the `:redir` capture next to
# it as fixture.out. With no arguments, every fixture under
# tests/all/fixtures/vimscript is recorded. Review the diff before
# committing: Viem's tests compare against these files.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
vim=${VIEM_VIM_ORACLE:-$(command -v vim)}
if [ "$#" -eq 0 ]; then
  set -- "$root"/tests/all/fixtures/vimscript/*.vim
fi
for fixture in "$@"; do
  directory=$(cd "$(dirname "$fixture")" && pwd)
  name=$(basename "$fixture" .vim)
  out="$directory/$name.out"
  rm -f "$out"
  (cd "$directory" && LC_ALL=en_US.UTF-8 LANG=en_US.UTF-8 "$vim" -u NONE -N -i NONE -es \
    -c "redir! > $out" -c "source $name.vim" -c "redir END" -c "qa!" </dev/null >/dev/null 2>&1) || true
  # Drop context headers (they contain local paths) and the alignment of
  # "line N:" so the recording is portable.
  sed -e '/^Error detected while processing/d' -e 's/^line  *\([0-9][0-9]*\):$/line \1:/' "$out" > "$out.tmp"
  mv "$out.tmp" "$out"
  echo "recorded $out"
done
